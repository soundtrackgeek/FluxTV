import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { command, destroy, init, listenEvents, setProperty, setVideoMarginRatio } from "tauri-plugin-libmpv-api";
import type { PlayInfo } from "../api";

export interface Track {
  id: number;
  type: "audio" | "sub" | "video";
  title?: string;
  lang?: string;
  codec?: string;
  selected?: boolean;
}

interface PlayerState {
  status: "idle" | "loading" | "playing" | "reconnecting" | "ended" | "error";
  error: string | null;
  paused: boolean;
  position: number;
  duration: number;
  seekable: boolean;
  tracks: Track[];
  aid: string;
  sid: string;
  videoCodec: string;
  audioCodec: string;
  width: number;
  height: number;
  fps: number;
}

const EMPTY: PlayerState = {
  status: "idle", error: null, paused: false, position: 0, duration: 0, seekable: false,
  tracks: [], aid: "auto", sid: "no", videoCodec: "", audioCodec: "", width: 0, height: 0, fps: 0,
};

// All native operations share a queue, including StrictMode cleanup and channel changes.
// An old initialization or destroy must never overtake a new loadfile.
let nativeQueue: Promise<unknown> = Promise.resolve();
function enqueue<T>(operation: () => Promise<T>): Promise<T> {
  const next = nativeQueue.then(operation);
  nativeQueue = next.catch(() => {});
  return next;
}

export function useMpv(
  playing: PlayInfo | null,
  surface: RefObject<HTMLDivElement | null>,
  layoutKey: string,
  volume: number,
  muted: boolean,
) {
  const [state, setState] = useState(EMPTY);
  const [ready, setReady] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const initializationRetry = useRef<{ url: string; startedAt: number } | null>(null);
  const controls = useRef<{ retry: () => void; run: (fn: () => Promise<void>) => void } | null>(null);
  const audio = useRef({ volume, muted });
  audio.current = { volume, muted };

  const updateBounds = useCallback(async () => {
    const rect = surface.current?.getBoundingClientRect();
    if (!rect || !rect.width || !rect.height) return;
    // A transparent floating player still has the browsing page beneath it.
    // Cut that page's painting out of the actual video rectangle as it moves.
    const main = surface.current?.closest(".app")?.querySelector<HTMLElement>(".main");
    if (main) {
      const page = main.getBoundingClientRect();
      for (const edge of ["left", "right", "top", "bottom"] as const) {
        const origin = edge === "left" || edge === "right" ? page.left : page.top;
        main.style.setProperty(`--native-video-${edge}`, `${rect[edge] - origin}px`);
      }
    }
    const clamp = (value: number) => Math.max(0, Math.min(1, value));
    await setVideoMarginRatio({
      left: clamp(rect.left / window.innerWidth), right: clamp(1 - rect.right / window.innerWidth),
      top: clamp(rect.top / window.innerHeight), bottom: clamp(1 - rect.bottom / window.innerHeight),
    });
  }, [surface]);

  const url = playing?.url;
  const live = playing?.channel.kind === "live";
  useEffect(() => {
    setReady(false);
    setState({ ...EMPTY, status: url ? "loading" : "idle" });
    if (!url) return;
    let closed = false;
    let initialized = false;
    let unlisten: (() => void) | undefined;
    let retryTimer: number | undefined;
    let retries = 0;
    let retryPending = false;
    let paused = false;
    let ended = false;
    let failed = false;
    let buffering = false;
    let lastPosition = -1;
    let lastAdvance = Date.now();
    let healthySince = Date.now();
    let lastPositionRender = 0;
    let startedAt = initializationRetry.current?.url === url
      ? initializationRetry.current.startedAt : playing?.requestedAt ?? performance.now();
    let firstFrameLogged = false;
    let loadStarted = false;

    const fail = (message: string) => {
      if (closed) return;
      failed = true;
      window.clearTimeout(retryTimer);
      setState((s) => ({ ...s, status: "error", error: message }));
    };
    const run = (fn: () => Promise<void>) => {
      void enqueue(async () => {
        if (!closed && initialized) await fn();
      }).catch(() => fail("The player could not complete this action. Try restarting playback."));
    };
    const load = async () => {
      await command("stop");
      if (closed) return;
      ended = false;
      failed = false;
      buffering = false;
      lastPosition = -1;
      lastAdvance = healthySince = Date.now();
      loadStarted = true;
      firstFrameLogged = false;
      // Use a short startup wait; after the first frame normal rebuffering resumes.
      if (live) {
        await setProperty("cache-pause-wait", 0.25);
        // An unusual stream may need more probing. Reconnects allow normal analysis.
        await setProperty("demuxer-lavf-analyzeduration", retries > 0 ? 0 : 1);
      }
      await setProperty("pause", false);
      await command("loadfile", [url, "replace"]);
    };
    const retry = (manual = false) => {
      if (closed || retryPending || !initialized) return;
      if (manual) retries = 0;
      if (!manual && retries >= 4) {
        fail("The stream stopped responding after several reconnect attempts. Retry or choose another channel.");
        return;
      }
      retryPending = true;
      startedAt = performance.now();
      retries += 1;
      setState((s) => ({ ...s, status: manual ? "loading" : "reconnecting", error: null }));
      retryTimer = window.setTimeout(() => {
        void enqueue(async () => {
          if (!closed) await load();
        }).catch(() => fail("The stream could not be reopened. Try again or choose another channel."))
          .finally(() => { retryPending = false; });
      }, manual ? 0 : 1200 * retries);
    };
    controls.current = { retry: () => retry(true), run };

    void enqueue(async () => {
      if (closed) return;
      unlisten = await listenEvents((event) => {
        if (closed) return;
        if (event.event === "playback-restart" && !ended && !failed) {
          if (loadStarted && !firstFrameLogged) {
            firstFrameLogged = true;
            const milliseconds = Math.round(performance.now() - startedAt);
            console.info(`[FluxTV] time to first frame: ${milliseconds} ms (Windows, playback-restart)`);
            if (live) run(() => setProperty("cache-pause-wait", 1));
          }
          setState((s) => ({ ...s, status: "playing", error: null }));
        } else if (event.event === "end-file") {
          if (event.reason === "stop" || event.reason === "quit" || event.reason === "redirect") return;
          ended = true;
          if (live) retry();
          else if (event.reason === "eof") setState((s) => ({ ...s, status: "ended", paused: true }));
          else fail("The movie could not be played. The provider may be offline or the file may be damaged.");
        } else if (event.event === "property-change") {
          const { name, data } = event;
          if (name === "time-pos") {
            if (typeof data !== "number" || !Number.isFinite(data)) return;
            if (data !== lastPosition) {
              lastPosition = data;
              lastAdvance = Date.now();
              if (lastAdvance - healthySince > 30_000) retries = 0;
            }
            if (Date.now() - lastPositionRender < 250) return;
            lastPositionRender = Date.now();
            setState((s) => ({ ...s, position: data }));
          } else if (name === "pause") {
            paused = data === true;
            lastAdvance = healthySince = Date.now();
            setState((s) => ({ ...s, paused }));
          } else if (name === "eof-reached" && !live) {
            ended = data === true;
            if (ended) setState((s) => ({ ...s, status: "ended", paused: true }));
          } else if (name === "paused-for-cache") {
            if (!ended && !failed && !retryPending) {
              const wasBuffering = buffering;
              buffering = data === true;
              if (data === true) healthySince = Date.now();
              // The initial false observation precedes the first decoded frame.
              // Only playback-restart or recovery from real buffering can hide loading.
              if (buffering || wasBuffering) {
                setState((s) => ({ ...s, status: buffering ? "loading" : "playing" }));
              }
            }
          } else if (name === "track-list") {
            setState((s) => ({ ...s, tracks: Array.isArray(data) ? data as Track[] : [] }));
          } else {
            const strings: Record<string, "videoCodec" | "audioCodec" | "aid" | "sid"> = {
              "video-codec": "videoCodec", "audio-codec-name": "audioCodec", aid: "aid", sid: "sid",
            };
            const numbers: Record<string, "duration" | "width" | "height" | "fps"> = {
              duration: "duration", width: "width", height: "height", "container-fps": "fps",
            };
            if (strings[name]) setState((s) => ({ ...s, [strings[name]]: String(data ?? "") }));
            else if (numbers[name]) setState((s) => ({ ...s, [numbers[name]]: typeof data === "number" ? data : 0 }));
            else if (name === "seekable") setState((s) => ({ ...s, seekable: data === true }));
          }
        }
      });
      if (closed) { unlisten(); return; }
      await init({
        initialOptions: {
          config: "no", "load-scripts": "no", ytdl: "no", osc: "no", terminal: "no",
          "input-default-bindings": "no", "input-vo-keyboard": "no",
          // The mpv window spans the app; keep subtitles inside the boxed video.
          "sub-use-margins": "no", "sub-scale-with-window": "no",
          vo: "gpu-next", hwdec: "auto-safe", idle: "yes", "force-window": "yes", "keep-open": live ? "no" : "yes",
          cache: "yes", "cache-secs": 3, "demuxer-max-bytes": 64 * 1024 * 1024,
          ...(live ? {
            "cache-pause-initial": "yes", "cache-pause-wait": 0.25,
            "demuxer-lavf-analyzeduration": 1,
          } : {}),
          volume: audio.current.volume * 100, mute: audio.current.muted,
        },
        observedProperties: [
          ["pause", "flag"], ["paused-for-cache", "flag"], ["time-pos", "double"],
          ["duration", "double"], ["seekable", "flag"], ["track-list", "node"],
          ["aid", "string"], ["sid", "string"], ["video-codec", "string"],
          ["audio-codec-name", "string"], ["width", "int64"], ["height", "int64"], ["container-fps", "double"],
          ["eof-reached", "flag"],
        ],
      });
      initialized = true;
      if (closed) return;
      await updateBounds();
      await load();
      setReady(true);
    }).catch(() => fail("The built-in player could not start. Reinstall FluxTV to restore its player libraries, or use an external player."));

    const watchdog = window.setInterval(() => {
      if (closed || !live || paused || failed || retryPending || !initialized) return;
      if (Date.now() - lastAdvance > 12_000) {
        healthySince = Date.now();
        retry();
      }
    }, 2000);
    return () => {
      closed = true;
      controls.current = null;
      unlisten?.();
      window.clearTimeout(retryTimer);
      window.clearInterval(watchdog);
      void enqueue(async () => {
        if (initialized) await destroy();
      }).catch(() => {});
    };
  }, [url, live, attempt, updateBounds]);

  useEffect(() => {
    if (!ready) return;
    controls.current?.run(async () => {
      await setProperty("volume", volume * 100);
      await setProperty("mute", muted);
    });
  }, [ready, volume, muted]);

  useEffect(() => {
    if (!ready || !surface.current) return;
    let frame = 0;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => controls.current?.run(updateBounds));
    };
    const observer = new ResizeObserver(update);
    observer.observe(surface.current);
    window.addEventListener("resize", update);
    update();
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", update);
    };
  }, [ready, surface, layoutKey, updateBounds]);

  return {
    ...state, ready,
    retry: () => {
      if (ready) controls.current?.retry();
      else {
        initializationRetry.current = url ? { url, startedAt: performance.now() } : null;
        setAttempt((n) => n + 1);
      }
    },
    togglePause: () => controls.current?.run(async () => {
      if (state.status === "ended") {
        await command("seek", [0, "absolute"]);
      }
      await setProperty("pause", !state.paused);
    }),
    seek: (seconds: number) => controls.current?.run(() => command("seek", [seconds, "absolute"])),
    selectTrack: (type: "aid" | "sid", id: string) => controls.current?.run(() => setProperty(type, id)),
  };
}
