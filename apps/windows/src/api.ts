// Typed wrappers around the Tauri commands exposed by src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ContentKind = "live" | "movie";
export type SourceKind = "xtream" | "m3u";

export interface Source {
  id: number;
  name: string;
  kind: SourceKind;
  url: string;
  username: string | null;
  epgUrl: string | null;
  createdAt: number;
  catalogUpdatedAt: number | null;
  epgUpdatedAt: number | null;
}

export interface NewSource {
  name: string;
  kind: SourceKind;
  url: string;
  username?: string | null;
  password?: string | null;
  epgUrl?: string | null;
}

export interface Category {
  id: string;
  name: string;
  kind: ContentKind;
  channelCount: number;
}

export interface Channel {
  id: string;
  kind: ContentKind;
  num: number;
  name: string;
  logo: string | null;
  categoryId: string;
  epgId: string | null;
  catchupDays: number;
  container: string | null;
  favorite: boolean;
}

export interface Programme {
  channel: string;
  start: number;
  stop: number;
  title: string;
  subtitle: string | null;
  description: string | null;
  category: string | null;
  icon: string | null;
  episode: string | null;
}

export interface NowNext {
  epgId: string;
  now: Programme | null;
  next: Programme | null;
}

export interface GuideRow {
  epgId: string;
  programmes: Programme[];
}

export interface AccountInfo {
  status: string;
  expiresAt: number | null;
  isTrial: boolean;
  activeConnections: number;
  maxConnections: number;
  allowedFormats: string[];
  serverTimezone: string | null;
  message: string | null;
}

export interface PlayInfo {
  url: string;
  mode: "mpegts" | "native";
  channel: Channel;
}

export interface ExternalPlayer {
  name: string;
  path: string;
}

export interface Progress {
  sourceId: number;
  stage: "catalog" | "epg";
  message: string;
  fraction: number | null;
  done: boolean;
}

export interface RefreshError {
  sourceId: number;
  stage: string;
  message: string;
}

export const api = {
  appInfo: () => invoke<{ version: string; coreVersion: string }>("app_info"),

  listSources: () => invoke<Source[]>("list_sources"),
  addSource: (input: NewSource) => invoke<Source>("add_source", { input }),
  updateSource: (id: number, input: NewSource) => invoke<Source>("update_source", { id, input }),
  deleteSource: (id: number) => invoke<void>("delete_source", { id }),
  accountInfo: (sourceId: number) => invoke<AccountInfo | null>("account_info", { sourceId }),
  refreshSource: (sourceId: number, force: boolean) => invoke<boolean>("refresh_source", { sourceId, force }),

  categories: (sourceId: number, kind: ContentKind) => invoke<Category[]>("get_categories", { sourceId, kind }),
  channels: (sourceId: number, kind: ContentKind, categoryId: string) =>
    invoke<Channel[]>("get_channels", { sourceId, kind, categoryId }),
  favorites: (sourceId: number, kind: ContentKind) => invoke<Channel[]>("get_favorites", { sourceId, kind }),
  recent: (sourceId: number, kind: ContentKind) => invoke<Channel[]>("get_recent", { sourceId, kind }),
  clearHistory: (sourceId: number) => invoke<void>("clear_history", { sourceId }),
  search: (sourceId: number, kind: ContentKind, query: string) =>
    invoke<Channel[]>("search_channels", { sourceId, kind, query }),
  toggleFavorite: (sourceId: number, kind: ContentKind, channelId: string) =>
    invoke<boolean>("toggle_favorite", { sourceId, kind, channelId }),

  nowNext: (sourceId: number, epgIds: string[]) => invoke<NowNext[]>("now_next", { sourceId, epgIds }),
  guide: (sourceId: number, epgIds: string[], from: number, to: number) =>
    invoke<GuideRow[]>("get_guide", { sourceId, epgIds, from, to }),

  play: (sourceId: number, kind: ContentKind, channelId: string) =>
    invoke<PlayInfo>("play", { sourceId, kind, channelId }),
  stopPlayback: () => invoke<void>("stop_playback"),
  externalPlayer: () => invoke<ExternalPlayer | null>("external_player"),
  openExternal: (sourceId: number, kind: ContentKind, channelId: string) =>
    invoke<string>("open_external", { sourceId, kind, channelId }),

  getSetting: (key: string) => invoke<string | null>("get_setting", { key }),
  setSetting: (key: string, value: string | null) => invoke<void>("set_setting", { key, value }),
};

export const events = {
  onProgress: (cb: (p: Progress) => void): Promise<UnlistenFn> => listen<Progress>("refresh-progress", (e) => cb(e.payload)),
  onError: (cb: (e: RefreshError) => void): Promise<UnlistenFn> => listen<RefreshError>("refresh-error", (e) => cb(e.payload)),
  onCatalogUpdated: (cb: (sourceId: number) => void): Promise<UnlistenFn> =>
    listen<number>("catalog-updated", (e) => cb(e.payload)),
  onEpgUpdated: (cb: (sourceId: number) => void): Promise<UnlistenFn> => listen<number>("epg-updated", (e) => cb(e.payload)),
};

/** Tauri rejects with a plain string; normalize anything into a message. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
