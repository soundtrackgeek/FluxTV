// User actions shared by several components.
import { useEffect, useState } from "react";
import { api, errorMessage, type Channel } from "./api";
import { useApp } from "./store";
import { useDebounced } from "./util";

export async function playChannel(channel: Channel, playlist?: Channel[]) {
  const { sourceId, setPlaying, showToast } = useApp.getState();
  if (!sourceId) return;
  const requestedAt = performance.now();
  try {
    const info = await api.play(sourceId, channel.kind, channel.id);
    setPlaying({ ...info, requestedAt }, playlist);
  } catch (e) {
    showToast(`Could not start ${channel.name}: ${errorMessage(e)}`, "error");
  }
}

/** Channel up/down within the list the current channel was started from. */
export function zap(delta: number) {
  const { playing, playlist } = useApp.getState();
  if (!playing || playlist.length === 0) return;
  const i = playlist.findIndex((c) => c.id === playing.channel.id);
  const next = playlist[(i + delta + playlist.length) % playlist.length];
  if (next) playChannel(next, playlist);
}

export async function stopPlayback() {
  useApp.getState().setPlaying(null);
  await api.stopPlayback().catch(() => {});
}

export async function toggleFavorite(channel: Channel): Promise<boolean | null> {
  const { sourceId, markFavorite, showToast } = useApp.getState();
  if (!sourceId) return null;
  try {
    const fav = await api.toggleFavorite(sourceId, channel.kind, channel.id);
    markFavorite(channel.id, fav);
    window.dispatchEvent(new CustomEvent("fluxtv:favorite", { detail: { id: channel.id, favorite: fav } }));
    return fav;
  } catch (e) {
    showToast(errorMessage(e), "error");
    return null;
  }
}

export async function openExternal(channel: Channel) {
  const { sourceId, showToast } = useApp.getState();
  if (!sourceId) return;
  // Free the provider connection first: many accounts allow only one stream at a time.
  await stopPlayback();
  try {
    const player = await api.openExternal(sourceId, channel.kind, channel.id);
    showToast(`Opened ${channel.name} in ${player}`);
  } catch (e) {
    showToast(errorMessage(e), "error");
  }
}

export function selectionTitle(): string {
  const { selection, query } = useApp.getState();
  if (query.trim()) return `Results for “${query.trim()}”`;
  if (selection.type === "favorites") return "Favorites";
  if (selection.type === "recent") return "Recently watched";
  return selection.name;
}

/** Channels for the current selection or search query. */
export function useChannels() {
  const { sourceId, kind, selection, query, catalogVersion } = useApp();
  const q = useDebounced(query.trim(), 160);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [loading, setLoading] = useState(false);
  const selKey = JSON.stringify(selection);

  useEffect(() => {
    if (!sourceId) return;
    let cancelled = false;
    setLoading(true);
    const load = q
      ? api.search(sourceId, kind, q)
      : selection.type === "favorites"
        ? api.favorites(sourceId, kind)
        : selection.type === "recent"
          ? api.recent(sourceId, kind)
          : api.channels(sourceId, kind, selection.id);
    load
      .then((c) => !cancelled && setChannels(c))
      .catch((e) => !cancelled && useApp.getState().showToast(errorMessage(e), "error"))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceId, kind, selKey, q, catalogVersion]);

  // Keep favorite stars in sync when toggled elsewhere (player, guide).
  useEffect(() => {
    const onFav = (e: Event) => {
      const { id, favorite } = (e as CustomEvent<{ id: string; favorite: boolean }>).detail;
      setChannels((list) => {
        if (!favorite && useApp.getState().selection.type === "favorites" && !useApp.getState().query.trim()) {
          return list.filter((c) => c.id !== id);
        }
        return list.map((c) => (c.id === id ? { ...c, favorite } : c));
      });
    };
    window.addEventListener("fluxtv:favorite", onFav);
    return () => window.removeEventListener("fluxtv:favorite", onFav);
  }, []);

  return { channels, loading, searching: q.length > 0 };
}
