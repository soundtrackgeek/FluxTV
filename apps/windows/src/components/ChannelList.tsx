import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ExternalLink, History, Star } from "lucide-react";
import type { Channel } from "../api";
import { openExternal, playChannel, toggleFavorite } from "../actions";
import { useApp, useEpg } from "../store";
import { cx, minutesLeft, progressOf, splitPrefix, useNow } from "../util";
import { Logo } from "./Logo";

const ROW_HEIGHT = 64;

export function ChannelList({ channels, loading, emptyHint }: { channels: Channel[]; loading: boolean; emptyHint: string }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const { sourceId, playing, epgVersion } = useApp();
  const epg = useEpg((s) => s.entries);
  const now = useNow();
  const isLive = channels[0]?.kind !== "movie";

  const virtualizer = useVirtualizer({
    count: channels.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  });
  const items = virtualizer.getVirtualItems();

  // Ask for now/next of the rows on screen only.
  const first = items[0]?.index ?? 0;
  const last = items[items.length - 1]?.index ?? -1;
  useEffect(() => {
    if (!sourceId || !isLive) return;
    const ids = new Set<string>();
    for (let i = first; i <= last; i++) {
      const id = channels[i]?.epgId;
      if (id) ids.add(id);
    }
    if (ids.size) useEpg.getState().request(sourceId, [...ids]);
  }, [sourceId, channels, first, last, isLive, epgVersion, Math.floor(now / 60)]);

  // Reset scroll when the list changes identity (new category/search).
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [channels.length > 0 ? channels[0].id + channels.length : ""]);

  if (!loading && channels.length === 0) {
    return <div className="empty">{emptyHint}</div>;
  }

  return (
    <div className="channel-scroll" ref={scrollRef}>
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {items.map((row) => {
          const ch = channels[row.index];
          const nn = ch.epgId ? epg[ch.epgId]?.data : undefined;
          const current = nn?.now && nn.now.stop > now ? nn.now : null;
          const { prefix, rest } = splitPrefix(ch.name);
          const isPlaying = playing?.channel.id === ch.id && playing.channel.kind === ch.kind;
          return (
            <div
              key={`${ch.kind}:${ch.id}`}
              className={cx("channel-row", isPlaying && "playing")}
              style={{ transform: `translateY(${row.start}px)`, height: ROW_HEIGHT }}
              onClick={() => playChannel(ch, channels)}
              onKeyDown={(e) => e.key === "Enter" && playChannel(ch, channels)}
              tabIndex={0}
              role="button"
              title={ch.name}
            >
              <Logo src={ch.logo} name={ch.name} className={ch.kind === "movie" ? "poster" : ""} />
              <div className="channel-main">
                <div className="channel-name">
                  {prefix && <span className="prefix">{prefix}</span>}
                  <span className="name">{rest}</span>
                  {ch.catchupDays > 0 && (
                    <span className="badge" title={`${ch.catchupDays} days catch-up available`}>
                      <History size={11} /> {ch.catchupDays}d
                    </span>
                  )}
                </div>
                {isLive && (
                  <div className="channel-epg">
                    {current ? (
                      <>
                        <span className="epg-title">{current.title}</span>
                        <span className="epg-left">{minutesLeft(current, now)} min left</span>
                      </>
                    ) : nn?.next ? (
                      <span className="epg-title muted">Next: {nn.next.title}</span>
                    ) : (
                      <span className="muted">{ch.epgId ? (nn ? "No programme info" : "") : "No guide"}</span>
                    )}
                  </div>
                )}
                {current && (
                  <div className="epg-progress">
                    <div style={{ width: `${progressOf(current, now) * 100}%` }} />
                  </div>
                )}
              </div>
              <div className="row-actions">
                <button
                  className="icon-btn"
                  title="Open in external player"
                  onClick={(e) => {
                    e.stopPropagation();
                    openExternal(ch);
                  }}
                >
                  <ExternalLink size={15} />
                </button>
                <button
                  className={cx("icon-btn", "star", ch.favorite && "on")}
                  title={ch.favorite ? "Remove from favorites" : "Add to favorites"}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleFavorite(ch);
                  }}
                >
                  <Star size={16} fill={ch.favorite ? "currentColor" : "none"} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
