import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Play, X } from "lucide-react";
import { api, type Channel, type Programme } from "../api";
import { playChannel } from "../actions";
import { useApp } from "../store";
import { cx, fmtDay, fmtRange, fmtTime, splitPrefix, useNow } from "../util";
import { Logo } from "./Logo";

const ROW = 58;
const CHANNEL_COL = 210;
const PX_PER_MIN = 5;
const HOURS_BACK = 2;
const HOURS_AHEAD = 22;

/** Classic EPG grid: channels down, time across. Only visible rows are fetched. */
export function Guide({ channels }: { channels: Channel[] }) {
  const { sourceId, epgVersion, playing } = useApp();
  const scrollRef = useRef<HTMLDivElement>(null);
  const now = useNow(30_000);
  const [rows, setRows] = useState<Record<string, Programme[]>>({});
  const [selected, setSelected] = useState<{ p: Programme; ch: Channel } | null>(null);

  // Fixed window anchored at the half hour, recomputed only when the guide opens.
  const [from] = useState(() => {
    const t = Math.floor(Date.now() / 1000);
    return t - (t % 1800) - HOURS_BACK * 3600;
  });
  const to = from + (HOURS_BACK + HOURS_AHEAD) * 3600;
  const width = ((to - from) / 60) * PX_PER_MIN;
  const x = (t: number) => ((Math.min(Math.max(t, from), to) - from) / 60) * PX_PER_MIN;

  const virtualizer = useVirtualizer({
    count: channels.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW,
    overscan: 6,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items[items.length - 1]?.index ?? -1;

  useEffect(() => setRows({}), [epgVersion, channels]);

  useEffect(() => {
    if (!sourceId) return;
    const missing = new Set<string>();
    for (let i = first; i <= last; i++) {
      const id = channels[i]?.epgId;
      if (id && !(id in rows)) missing.add(id);
    }
    if (missing.size === 0) return;
    const t = window.setTimeout(() => {
      api.guide(sourceId, [...missing], from, to).then((res) => {
        setRows((prev) => {
          const next = { ...prev };
          for (const r of res) next[r.epgId] = r.programmes;
          return next;
        });
      });
    }, 60);
    return () => window.clearTimeout(t);
  }, [sourceId, channels, first, last, rows, from, to]);

  // Start scrolled to "now".
  useLayoutEffect(() => {
    scrollRef.current?.scrollTo({ left: Math.max(0, x(Date.now() / 1000) - 120) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const hours = useMemo(() => {
    const marks: number[] = [];
    for (let t = from; t < to; t += 1800) marks.push(t);
    return marks;
  }, [from, to]);

  if (channels.length === 0) return <div className="empty">No channels to show in the guide.</div>;

  return (
    <div className="guide">
      <div className="guide-scroll" ref={scrollRef}>
        <div style={{ width: CHANNEL_COL + width, height: virtualizer.getTotalSize() + 36, position: "relative" }}>
          <div className="guide-timebar" style={{ width: CHANNEL_COL + width }}>
            <div className="guide-corner" style={{ width: CHANNEL_COL }}>
              {fmtDay(now)}
            </div>
            {hours.map((t) => (
              <div key={t} className="guide-tick" style={{ left: CHANNEL_COL + x(t) }}>
                {fmtTime(t)}
              </div>
            ))}
          </div>
          <div className="guide-now" style={{ left: CHANNEL_COL + x(now), height: virtualizer.getTotalSize() }} />
          {items.map((row) => {
            const ch = channels[row.index];
            const progs = ch.epgId ? rows[ch.epgId] : undefined;
            const isPlaying = playing?.channel.id === ch.id;
            return (
              <div key={ch.id} className="guide-row" style={{ transform: `translateY(${row.start + 36}px)`, height: ROW }}>
                <button
                  className={cx("guide-channel", isPlaying && "playing")}
                  style={{ width: CHANNEL_COL }}
                  onClick={() => playChannel(ch, channels)}
                  title={`Watch ${ch.name}`}
                >
                  <Logo src={ch.logo} name={ch.name} />
                  <span>{splitPrefix(ch.name).rest}</span>
                </button>
                <div className="guide-track" style={{ left: CHANNEL_COL, width }}>
                  {!ch.epgId && <div className="guide-none">No guide data</div>}
                  {ch.epgId && progs && progs.length === 0 && <div className="guide-none">No programmes</div>}
                  {progs?.map((p) => {
                    const left = x(p.start);
                    const w = Math.max(2, x(p.stop) - left - 2);
                    const live = p.start <= now && p.stop > now;
                    const past = p.stop <= now;
                    return (
                      <button
                        key={p.start}
                        className={cx("guide-prog", live && "live", past && "past")}
                        style={{ left, width: w }}
                        onClick={() => setSelected({ p, ch })}
                        onDoubleClick={() => playChannel(ch, channels)}
                        title={`${p.title}\n${fmtRange(p)}`}
                      >
                        <span className="gp-title">{p.title}</span>
                        {w > 90 && <span className="gp-time">{fmtRange(p)}</span>}
                      </button>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {selected && (
        <div className="guide-detail">
          <button className="icon-btn close" onClick={() => setSelected(null)}>
            <X size={16} />
          </button>
          <div className="muted small">
            {splitPrefix(selected.ch.name).rest} · {fmtDay(selected.p.start)} {fmtRange(selected.p)}
          </div>
          <h3>{selected.p.title}</h3>
          {selected.p.subtitle && <div className="np-sub">{selected.p.subtitle}</div>}
          {selected.p.description && <p className="np-desc">{selected.p.description}</p>}
          <button className="btn primary" onClick={() => playChannel(selected.ch, channels)}>
            <Play size={15} /> Watch {splitPrefix(selected.ch.name).rest}
          </button>
        </div>
      )}
    </div>
  );
}
