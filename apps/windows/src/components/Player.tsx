import { useCallback, useEffect, useRef, useState } from "react";
import mpegts from "mpegts.js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlertTriangle,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  Loader2,
  Maximize,
  Minimize,
  Pause,
  Play,
  RotateCw,
  Star,
  Tv,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import type { PlayInfo } from "../api";
import { openExternal, stopPlayback, toggleFavorite, zap } from "../actions";
import { useApp, useEpg } from "../store";
import { cx, fmtRange, progressOf, splitPrefix, useNow } from "../util";

type Status = "idle" | "loading" | "playing" | "reconnecting" | "error";

interface MediaInfo {
  videoCodec?: string;
  audioCodec?: string;
  width?: number;
  height?: number;
  fps?: number;
}

const MAX_RETRIES = 4;
const STALL_TIMEOUT_MS = 12_000;

// Codecs WebView2's Media Source Extensions can't decode; mpegts.js would otherwise play silently.
function unsupportedAudio(codec?: string): string | null {
  if (!codec) return null;
  const c = codec.toLowerCase();
  if ((c.startsWith("ac-3") || c.startsWith("ec-3")) && !MediaSource.isTypeSupported(`audio/mp4; codecs="${c}"`)) {
    return c.startsWith("ec-3") ? "E-AC-3 (Dolby Digital Plus)" : "AC-3 (Dolby Digital)";
  }
  return null;
}

function describe(mi: MediaInfo | null): string {
  if (!mi) return "";
  const parts: string[] = [];
  if (mi.width && mi.height) parts.push(`${mi.width}×${mi.height}`);
  if (mi.fps) parts.push(`${Math.round(mi.fps)} fps`);
  if (mi.videoCodec) parts.push(mi.videoCodec.split(".")[0].replace("avc1", "H.264").replace(/hvc1|hev1/, "HEVC"));
  if (mi.audioCodec) parts.push(mi.audioCodec.split(".")[0].replace("mp4a", "AAC").replace("ac-3", "AC-3").replace("ec-3", "E-AC-3"));
  return parts.join(" · ");
}

export function Player() {
  const { playing, fullscreen } = useApp();
  const videoRef = useRef<HTMLVideoElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const mpegtsRef = useRef<mpegts.Player | null>(null);
  const retriesRef = useRef(0);
  const retryTimer = useRef<number | undefined>(undefined);
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);
  const [mediaInfo, setMediaInfo] = useState<MediaInfo | null>(null);
  const [paused, setPaused] = useState(false);
  const [muted, setMuted] = useState(() => localStorage.getItem("fluxtv.muted") === "1");
  const [volume, setVolume] = useState(() => Number(localStorage.getItem("fluxtv.volume") ?? "1"));
  const [controlsVisible, setControlsVisible] = useState(true);
  const hideTimer = useRef<number | undefined>(undefined);

  const destroy = useCallback(() => {
    window.clearTimeout(retryTimer.current);
    const p = mpegtsRef.current;
    mpegtsRef.current = null;
    if (p) {
      try {
        p.pause();
        p.unload();
        p.detachMediaElement();
        p.destroy();
      } catch {
        /* already torn down */
      }
    }
    const v = videoRef.current;
    if (v) {
      v.removeAttribute("src");
      v.load();
    }
  }, []);

  const start = useCallback(
    (info: PlayInfo) => {
      destroy();
      const video = videoRef.current;
      if (!video) return;
      setError(null);
      setStatus(retriesRef.current > 0 ? "reconnecting" : "loading");

      if (info.mode === "native") {
        video.src = info.url;
        video.play().catch(() => {});
        return;
      }
      if (!mpegts.getFeatureList().mseLivePlayback) {
        setStatus("error");
        setError("This system does not support Media Source Extensions.");
        return;
      }
      const player = mpegts.createPlayer(
        { type: "mpegts", isLive: true, url: info.url },
        {
          enableWorker: true,
          enableStashBuffer: true,
          stashInitialSize: 384 * 1024,
          lazyLoad: false,
          autoCleanupSourceBuffer: true,
          autoCleanupMaxBackwardDuration: 60,
          autoCleanupMinBackwardDuration: 30,
          // IPTV "RAW" feeds are high bitrate and arrive in bursts; a thin buffer stutters.
          // Only chase the live edge once we fall well behind, and keep a healthy cushion.
          liveBufferLatencyChasing: true,
          liveBufferLatencyMaxLatency: 10,
          liveBufferLatencyMinRemain: 3,
          fixAudioTimestampGap: true,
        },
      );
      player.on(mpegts.Events.MEDIA_INFO, (mi: MediaInfo) => setMediaInfo(mi));
      player.on(mpegts.Events.ERROR, (type: string, detail: string, data: { msg?: string; code?: number }) => {
        console.warn("mpegts error", type, detail, data);
        const fatalMedia =
          detail === mpegts.ErrorDetails.MEDIA_CODEC_UNSUPPORTED || detail === mpegts.ErrorDetails.MEDIA_FORMAT_UNSUPPORTED;
        if (!fatalMedia && retriesRef.current < MAX_RETRIES) {
          retriesRef.current += 1;
          setStatus("reconnecting");
          retryTimer.current = window.setTimeout(() => start(info), 1200 * retriesRef.current);
          return;
        }
        setStatus("error");
        setError(
          fatalMedia
            ? "This channel uses a format the built-in player can't decode."
            : type === mpegts.ErrorTypes.NETWORK_ERROR
              ? `The stream could not be loaded${data?.code ? ` (HTTP ${data.code})` : ""}. The channel may be offline, or another device is using your connection.`
              : `Playback failed: ${data?.msg ?? detail}`,
        );
      });
      if (import.meta.env.DEV) {
        // Debug hook for inspecting playback from DevTools.
        const w = window as unknown as { __flux: Record<string, unknown> };
        w.__flux = { player, stats: null, logs: [] };
        player.on(mpegts.Events.STATISTICS_INFO, (s: unknown) => (w.__flux.stats = s));
      }
      mpegtsRef.current = player;
      player.attachMediaElement(video);
      player.load();
      video.play().catch(() => {
        /* autoplay promise rejects if the stream is replaced quickly; harmless */
      });
    },
    [destroy],
  );

  // Start/stop when the selected stream changes.
  useEffect(() => {
    retriesRef.current = 0;
    setMediaInfo(null);
    if (playing) start(playing);
    else {
      destroy();
      setStatus("idle");
    }
    return destroy;
  }, [playing?.url]); // eslint-disable-line react-hooks/exhaustive-deps

  // Video element events.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const onPlaying = () => {
      setStatus("playing");
      setPaused(false);
      retriesRef.current = 0;
    };
    const onWaiting = () => setStatus((s) => (s === "playing" ? "loading" : s));
    const onPause = () => setPaused(true);
    const onError = () => {
      if (playing?.mode === "native") {
        setStatus("error");
        setError("This file format isn't supported by the built-in player.");
      }
    };
    v.addEventListener("playing", onPlaying);
    v.addEventListener("waiting", onWaiting);
    v.addEventListener("pause", onPause);
    v.addEventListener("error", onError);
    return () => {
      v.removeEventListener("playing", onPlaying);
      v.removeEventListener("waiting", onWaiting);
      v.removeEventListener("pause", onPause);
      v.removeEventListener("error", onError);
    };
  }, [playing?.mode]);

  // Stall watchdog: live streams sometimes freeze without an error event.
  useEffect(() => {
    if (!playing || playing.mode !== "mpegts") return;
    let lastTime = -1;
    let lastChange = Date.now();
    const t = window.setInterval(() => {
      const v = videoRef.current;
      if (!v || v.paused || status === "error") {
        lastChange = Date.now();
        return;
      }
      if (v.currentTime !== lastTime) {
        lastTime = v.currentTime;
        lastChange = Date.now();
      } else if (Date.now() - lastChange > STALL_TIMEOUT_MS && retriesRef.current < MAX_RETRIES) {
        retriesRef.current += 1;
        lastChange = Date.now();
        start(playing);
      }
    }, 2000);
    return () => window.clearInterval(t);
  }, [playing, status, start]);

  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = muted;
    v.volume = volume;
    localStorage.setItem("fluxtv.muted", muted ? "1" : "0");
    localStorage.setItem("fluxtv.volume", String(volume));
  }, [muted, volume]);

  const setFullscreen = useCallback(async (on: boolean) => {
    useApp.getState().setFullscreen(on);
    try {
      await getCurrentWindow().setFullscreen(on);
    } catch (e) {
      console.warn("fullscreen", e);
    }
  }, []);

  const togglePause = useCallback(() => {
    const v = videoRef.current;
    if (!v || !playing) return;
    if (v.paused) {
      // For live streams, jump back to the live edge when resuming.
      if (playing.mode === "mpegts" && v.buffered.length) {
        v.currentTime = Math.max(v.currentTime, v.buffered.end(v.buffered.length - 1) - 1);
      }
      v.play().catch(() => {});
    } else v.pause();
  }, [playing]);

  // Keyboard shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT") return;
      const fs = useApp.getState().fullscreen;
      switch (e.key) {
        case "f":
        case "F":
          if (playing) setFullscreen(!fs);
          break;
        case "Escape":
          if (fs) setFullscreen(false);
          break;
        case "m":
        case "M":
          setMuted((m) => !m);
          break;
        case " ":
          if (playing) {
            e.preventDefault();
            togglePause();
          }
          break;
        case "PageUp":
          zap(-1);
          break;
        case "PageDown":
          zap(1);
          break;
        case "ArrowUp":
          if (fs) zap(-1);
          else return;
          break;
        case "ArrowDown":
          if (fs) zap(1);
          else return;
          break;
        default:
          return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [playing, setFullscreen, togglePause]);

  const poke = () => {
    setControlsVisible(true);
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setControlsVisible(false), 3000);
  };

  const channel = playing?.channel;
  const epg = useEpg((s) => (channel?.epgId ? s.entries[channel.epgId]?.data : undefined));
  const now = useNow(15_000);
  const current = epg?.now && epg.now.stop > now ? epg.now : null;
  const badAudio = unsupportedAudio(mediaInfo?.audioCodec);
  const title = channel ? splitPrefix(channel.name).rest : "";

  return (
    <div
      ref={containerRef}
      className={cx("player", fullscreen && "fullscreen", !controlsVisible && fullscreen && "hide-cursor")}
      onMouseMove={poke}
      onMouseLeave={() => setControlsVisible(false)}
      onDoubleClick={() => playing && setFullscreen(!fullscreen)}
    >
      <video
        ref={videoRef}
        className="video"
        playsInline
        // Movies get native controls (seek bar); live TV uses the custom overlay.
        controls={playing?.mode === "native"}
        onClick={playing?.mode === "native" ? undefined : togglePause}
      />

      {!playing && (
        <div className="player-placeholder">
          <Tv size={42} strokeWidth={1.2} />
          <div>Pick a channel to start watching</div>
          <div className="muted small">Double-click or press F for fullscreen · PgUp/PgDn to zap</div>
        </div>
      )}

      {playing && (status === "loading" || status === "reconnecting") && (
        <div className="player-center">
          <Loader2 size={36} className="spin" />
          <span>{status === "reconnecting" ? "Reconnecting…" : "Tuning in…"}</span>
        </div>
      )}

      {playing && status === "error" && channel && (
        <div className="player-center error">
          <AlertTriangle size={34} />
          <p>{error}</p>
          <div className="row gap">
            <button
              className="btn"
              onClick={() => {
                retriesRef.current = 0;
                start(playing);
              }}
            >
              <RotateCw size={15} /> Retry
            </button>
            <button className="btn primary" onClick={() => openExternal(channel)}>
              <ExternalLink size={15} /> Open in external player
            </button>
          </div>
        </div>
      )}

      {playing && channel && (
        <div className={cx("player-chrome", (controlsVisible || !fullscreen) && "visible")}>
          <div className="chrome-top">
            <div className="chrome-title">
              <strong>{title}</strong>
              {current && (
                <span>
                  {current.title} · {fmtRange(current)}
                </span>
              )}
            </div>
            <button className="icon-btn" title="Close player" onClick={() => (fullscreen ? setFullscreen(false) : stopPlayback())}>
              <X size={18} />
            </button>
          </div>

          {badAudio && status === "playing" && (
            <div className="audio-warning">
              <AlertTriangle size={14} /> Audio is {badAudio}, which the built-in player can't decode.
              <button className="link" onClick={() => openExternal(channel)}>
                Open in external player
              </button>
            </div>
          )}

          <div className={cx("chrome-bottom", playing.mode === "native" && "native")}>
            {current && (
              <div className="chrome-progress">
                <div style={{ width: `${progressOf(current, now) * 100}%` }} />
              </div>
            )}
            <div className="chrome-controls">
              <button className="icon-btn" onClick={togglePause} title="Play/pause (Space)">
                {paused ? <Play size={18} /> : <Pause size={18} />}
              </button>
              <button className="icon-btn" onClick={() => zap(-1)} title="Previous channel (PgUp)">
                <ChevronUp size={18} />
              </button>
              <button className="icon-btn" onClick={() => zap(1)} title="Next channel (PgDn)">
                <ChevronDown size={18} />
              </button>
              <button className="icon-btn" onClick={() => setMuted(!muted)} title="Mute (M)">
                {muted || volume === 0 ? <VolumeX size={18} /> : <Volume2 size={18} />}
              </button>
              <input
                className="volume"
                type="range"
                min={0}
                max={1}
                step={0.02}
                value={muted ? 0 : volume}
                onChange={(e) => {
                  setVolume(Number(e.target.value));
                  setMuted(false);
                }}
              />
              <span className="media-info">{status === "playing" ? describe(mediaInfo) : ""}</span>
              <span className="spacer" />
              <button
                className={cx("icon-btn", "star", channel.favorite && "on")}
                onClick={() => toggleFavorite(channel)}
                title="Favorite"
              >
                <Star size={17} fill={channel.favorite ? "currentColor" : "none"} />
              </button>
              <button className="icon-btn" onClick={() => openExternal(channel)} title="Open in external player">
                <ExternalLink size={17} />
              </button>
              <button className="icon-btn" onClick={() => setFullscreen(!fullscreen)} title="Fullscreen (F)">
                {fullscreen ? <Minimize size={18} /> : <Maximize size={18} />}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
