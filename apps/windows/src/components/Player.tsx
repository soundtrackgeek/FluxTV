import { useCallback, useEffect, useRef, useState } from "react";
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
import { useMpv, type Track } from "../hooks/useMpv";
import { openExternal, stopPlayback, toggleFavorite, zap } from "../actions";
import { useApp, useEpg } from "../store";
import { cx, fmtRange, progressOf, splitPrefix, useNow } from "../util";

function clock(seconds: number): string {
  const n = Math.max(0, Math.floor(seconds));
  return `${Math.floor(n / 3600) ? `${Math.floor(n / 3600)}:` : ""}${String(Math.floor(n / 60) % 60).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}`;
}

function trackLabel(track: Track): string {
  return [track.title || `Track ${track.id}`, track.lang, track.codec].filter(Boolean).join(" · ");
}

export function Player() {
  const { playing, fullscreen, screen, display } = useApp();
  const containerRef = useRef<HTMLDivElement>(null);
  const [muted, setMuted] = useState(() => localStorage.getItem("fluxtv.muted") === "1");
  const [volume, setVolume] = useState(() => {
    const saved = Number(localStorage.getItem("fluxtv.volume") ?? "1");
    return Number.isFinite(saved) ? Math.max(0, Math.min(1, saved)) : 1;
  });
  const [controlsVisible, setControlsVisible] = useState(true);
  const hideTimer = useRef<number | undefined>(undefined);
  const player = useMpv(playing, containerRef, `${fullscreen}/${screen}/${display}`, volume, muted);
  const { status, error, paused, togglePause } = player;
  const movie = playing?.channel.kind === "movie";
  const audioTracks = player.tracks.filter((t) => t.type === "audio");
  const subTracks = player.tracks.filter((t) => t.type === "sub");
  const description = [
    player.width && player.height ? `${player.width}×${player.height}` : "",
    player.fps ? `${Math.round(player.fps)} fps` : "",
    player.videoCodec, player.audioCodec.toUpperCase(),
  ].filter(Boolean).join(" · ");

  useEffect(() => {
    localStorage.setItem("fluxtv.muted", muted ? "1" : "0");
    localStorage.setItem("fluxtv.volume", String(volume));
  }, [muted, volume]);

  useEffect(() => () => window.clearTimeout(hideTimer.current), []);

  const setFullscreen = useCallback(async (on: boolean) => {
    useApp.getState().setFullscreen(on);
    try {
      await getCurrentWindow().setFullscreen(on);
    } catch (e) {
      console.warn("fullscreen", e);
    }
  }, []);

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
  const title = channel ? splitPrefix(channel.name).rest : "";

  return (
    <div
      ref={containerRef}
      className={cx("player", player.ready && "native-active", fullscreen && "fullscreen", !controlsVisible && fullscreen && "hide-cursor")}
      onMouseMove={poke}
      onMouseLeave={() => setControlsVisible(false)}
      onDoubleClick={() => playing && setFullscreen(!fullscreen)}
    >
      <div className="video-surface" onClick={togglePause} aria-label="Video surface" />

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
              onClick={player.retry}
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

          <div className="chrome-bottom">
            {movie && player.seekable && player.duration > 0 && (
              <div className="movie-seek">
                <span>{clock(player.position)}</span>
                <input aria-label="Seek movie" type="range" min={0} max={player.duration} step={1}
                  value={player.position} onChange={(e) => player.seek(Number(e.target.value))} />
                <span>{clock(player.duration)}</span>
              </div>
            )}
            {(audioTracks.length > 1 || subTracks.length > 0) && (
              <div className="track-controls">
                {audioTracks.length > 1 && (
                  <label>Audio <select aria-label="Audio track" value={player.aid}
                    onChange={(e) => player.selectTrack("aid", e.target.value)}>
                    <option value="auto">Automatic</option>
                    {audioTracks.map((t) => <option key={t.id} value={String(t.id)}>{trackLabel(t)}</option>)}
                  </select></label>
                )}
                {subTracks.length > 0 && (
                  <label>Subtitles <select aria-label="Subtitle track" value={player.sid}
                    onChange={(e) => player.selectTrack("sid", e.target.value)}>
                    <option value="no">Off</option>
                    <option value="auto">Automatic</option>
                    {subTracks.map((t) => <option key={t.id} value={String(t.id)}>{trackLabel(t)}</option>)}
                  </select></label>
                )}
              </div>
            )}
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
                aria-label="Volume"
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
              <span className="media-info">{status === "playing" ? description : ""}</span>
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
