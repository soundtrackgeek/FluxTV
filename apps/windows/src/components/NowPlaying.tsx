import { useEffect, useState } from "react";
import { api, type Programme } from "../api";
import { useApp } from "../store";
import { cx, fmtDay, fmtRange, fmtTime, progressOf, splitPrefix, useNow } from "../util";
import { Logo } from "./Logo";

/** Details for the playing channel: current programme and what's coming up. */
export function NowPlaying() {
  const { playing, sourceId, epgVersion } = useApp();
  const [upcoming, setUpcoming] = useState<Programme[]>([]);
  const [expanded, setExpanded] = useState<number | null>(null);
  const now = useNow();
  const channel = playing?.channel;
  const nowBucket = Math.floor(now / 300);

  useEffect(() => {
    setExpanded(null);
    if (!sourceId || !channel?.epgId) {
      setUpcoming([]);
      return;
    }
    let cancelled = false;
    const from = Math.floor(Date.now() / 1000);
    api
      .guide(sourceId, [channel.epgId], from, from + 18 * 3600)
      .then((rows) => !cancelled && setUpcoming(rows[0]?.programmes ?? []))
      .catch(() => !cancelled && setUpcoming([]));
    return () => {
      cancelled = true;
    };
  }, [sourceId, channel?.epgId, epgVersion, nowBucket]);

  if (!channel) return null;
  const { prefix, rest } = splitPrefix(channel.name);
  const current = upcoming.find((p) => p.start <= now && p.stop > now);

  return (
    <section className="now-playing">
      <header className="np-header">
        <Logo src={channel.logo} name={channel.name} />
        <div>
          <div className="np-channel">
            {prefix && <span className="prefix">{prefix}</span>} {rest}
          </div>
          {current ? (
            <div className="np-title">{current.title}</div>
          ) : (
            <div className="muted">{channel.kind === "movie" ? "Movie" : channel.epgId ? "No programme information" : "No guide for this channel"}</div>
          )}
        </div>
      </header>
      {current && (
        <div className="np-current">
          <div className="np-meta">
            {fmtRange(current)}
            {current.episode && <span className="pill">{current.episode}</span>}
            {current.category && <span className="pill">{current.category}</span>}
          </div>
          <div className="epg-progress big">
            <div style={{ width: `${progressOf(current, now) * 100}%` }} />
          </div>
          {current.subtitle && <div className="np-sub">{current.subtitle}</div>}
          {current.description && <p className="np-desc">{current.description}</p>}
        </div>
      )}
      {upcoming.filter((p) => p.start > now).length > 0 && (
        <>
          <div className="section-title">Up next</div>
          <ul className="upcoming">
            {upcoming
              .filter((p) => p.start > now)
              .slice(0, 20)
              .map((p) => (
                <li
                  key={p.start}
                  className={cx(expanded === p.start && "open")}
                  onClick={() => setExpanded(expanded === p.start ? null : p.start)}
                >
                  <span className="time">
                    {fmtDay(p.start) !== fmtDay(now) && <small>{fmtDay(p.start)} </small>}
                    {fmtTime(p.start)}
                  </span>
                  <div>
                    <div className="up-title">{p.title}</div>
                    {expanded === p.start && p.description && <p className="np-desc">{p.description}</p>}
                  </div>
                </li>
              ))}
          </ul>
        </>
      )}
    </section>
  );
}
