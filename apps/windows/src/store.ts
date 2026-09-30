// Global UI state (zustand). Server data stays in SQLite; this holds only what the UI needs.
import { create } from "zustand";
import { api, type Channel, type ContentKind, type NowNext, type PlayInfo, type Progress, type Source } from "./api";

export type Selection =
  | { type: "favorites" }
  | { type: "recent" }
  | { type: "category"; id: string; name: string };

export type Screen = "browse" | "settings";
export type Display = "list" | "guide";

interface AppState {
  sources: Source[];
  sourceId: number | null;
  kind: ContentKind;
  selection: Selection;
  query: string;
  screen: Screen;
  display: Display;
  playing: PlayInfo | null;
  /** The list the current channel was started from, for channel up/down. */
  playlist: Channel[];
  fullscreen: boolean;
  /** Bumped when the catalog/guide changes so views reload. */
  catalogVersion: number;
  epgVersion: number;
  progress: Record<string, Progress>;
  toast: { message: string; tone: "info" | "error" } | null;

  loadSources: () => Promise<Source[]>;
  setSource: (id: number | null) => void;
  setKind: (kind: ContentKind) => void;
  select: (s: Selection) => void;
  setQuery: (q: string) => void;
  setScreen: (s: Screen) => void;
  setDisplay: (d: Display) => void;
  setPlaying: (p: PlayInfo | null, playlist?: Channel[]) => void;
  setFullscreen: (f: boolean) => void;
  bumpCatalog: () => void;
  bumpEpg: () => void;
  setProgress: (p: Progress) => void;
  showToast: (message: string, tone?: "info" | "error") => void;
  markFavorite: (channelId: string, favorite: boolean) => void;
}

const SOURCE_KEY = "fluxtv.sourceId";
const SELECTION_KEY = "fluxtv.selection";

function loadSelection(): Selection {
  try {
    const s = JSON.parse(localStorage.getItem(SELECTION_KEY) ?? "null");
    if (s && (s.type === "favorites" || s.type === "recent" || s.type === "category")) return s;
  } catch {
    /* ignore */
  }
  return { type: "favorites" };
}

let toastTimer: number | undefined;

export const useApp = create<AppState>((set, get) => ({
  sources: [],
  sourceId: Number(localStorage.getItem(SOURCE_KEY)) || null,
  kind: "live",
  selection: loadSelection(),
  query: "",
  screen: "browse",
  display: "list",
  playing: null,
  playlist: [],
  fullscreen: false,
  catalogVersion: 0,
  epgVersion: 0,
  progress: {},
  toast: null,

  loadSources: async () => {
    const sources = await api.listSources();
    const current = get().sourceId;
    const sourceId = sources.some((s) => s.id === current) ? current : (sources[0]?.id ?? null);
    set({ sources, sourceId });
    if (sourceId) localStorage.setItem(SOURCE_KEY, String(sourceId));
    return sources;
  },
  setSource: (id) => {
    if (id) localStorage.setItem(SOURCE_KEY, String(id));
    set({ sourceId: id, selection: { type: "favorites" }, query: "", catalogVersion: get().catalogVersion + 1 });
  },
  setKind: (kind) => set({ kind, selection: { type: "favorites" }, query: "", display: "list" }),
  select: (selection) => {
    localStorage.setItem(SELECTION_KEY, JSON.stringify(selection));
    set({ selection, query: "", screen: "browse" });
  },
  setQuery: (query) => set({ query, screen: "browse" }),
  setScreen: (screen) => set({ screen }),
  setDisplay: (display) => set({ display }),
  setPlaying: (playing, playlist) => set(playlist ? { playing, playlist } : { playing }),
  setFullscreen: (fullscreen) => set({ fullscreen }),
  bumpCatalog: () => set({ catalogVersion: get().catalogVersion + 1 }),
  bumpEpg: () => set({ epgVersion: get().epgVersion + 1 }),
  setProgress: (p) => set({ progress: { ...get().progress, [`${p.sourceId}:${p.stage}`]: p } }),
  showToast: (message, tone = "info") => {
    window.clearTimeout(toastTimer);
    set({ toast: { message, tone } });
    toastTimer = window.setTimeout(() => set({ toast: null }), tone === "error" ? 7000 : 3500);
  },
  markFavorite: (channelId, favorite) => {
    const { playing, playlist } = get();
    set({
      playing: playing && playing.channel.id === channelId ? { ...playing, channel: { ...playing.channel, favorite } } : playing,
      playlist: playlist.map((c) => (c.id === channelId ? { ...c, favorite } : c)),
    });
  },
}));

// ----- EPG cache --------------------------------------------------------------------------

interface EpgState {
  entries: Record<string, { data: NowNext; fetchedAt: number }>;
  request: (sourceId: number, epgIds: string[]) => void;
  clear: () => void;
}

const EPG_TTL_MS = 60_000;
const inflight = new Set<string>();
let pending = new Set<string>();
let pendingSource = 0;
let flushTimer: number | undefined;

/** Now/next data for channels, fetched lazily in batches for whatever is on screen. */
export const useEpg = create<EpgState>((set, get) => ({
  entries: {},
  request: (sourceId, epgIds) => {
    const now = Date.now();
    const { entries } = get();
    for (const id of epgIds) {
      const e = entries[id];
      const stale = !e || now - e.fetchedAt > EPG_TTL_MS || (e.data.now && e.data.now.stop * 1000 < now);
      if (stale && !inflight.has(id)) pending.add(id);
    }
    pendingSource = sourceId;
    if (pending.size === 0) return;
    window.clearTimeout(flushTimer);
    flushTimer = window.setTimeout(async () => {
      const ids = [...pending];
      pending = new Set();
      ids.forEach((id) => inflight.add(id));
      try {
        const result = await api.nowNext(pendingSource, ids);
        const fetchedAt = Date.now();
        const next = { ...get().entries };
        for (const r of result) next[r.epgId] = { data: r, fetchedAt };
        set({ entries: next });
      } catch (e) {
        console.warn("now/next failed", e);
      } finally {
        ids.forEach((id) => inflight.delete(id));
      }
    }, 80);
  },
  clear: () => set({ entries: {} }),
}));
