import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronLeft, ChevronRight, Play, X } from "lucide-react";
import { api, type Channel, type Programme } from "../api";
import { playChannel } from "../actions";
import { useApp } from "../store";
import { cx, fmtDay, fmtRange, fmtTime, splitPrefix, useNow } from "../util";
import {
  CHANNEL_COL,
  EXTEND,
  HALF_HOUR,
  PX_PER_MIN,
  guideWindow,
  localDay,
  localTime,
  programmeAt,
  programmeCategory,
  visibleRange,
} from "../guide";
import { Logo } from "./Logo";

const ROW = 58;
type Focus = { channel: number; time: number; channelButton: boolean };

export function Guide({ channels }: { channels: Channel[] }) {
  const sourceId = useApp((s) => s.sourceId);
  const epgVersion = useApp((s) => s.epgVersion);
  // Provider changes start a new session; guide refreshes preserve date and scroll position.
  return (
    <GuideGrid
      key={sourceId}
      channels={channels}
      sourceId={sourceId}
      epgVersion={epgVersion}
    />
  );
}

function GuideGrid({
  channels,
  sourceId,
  epgVersion,
}: {
  channels: Channel[];
  sourceId: number | null;
  epgVersion: number;
}) {
  const playing = useApp((s) => s.playing);
  const scrollRef = useRef<HTMLDivElement>(null);
  const now = useNow(30_000);
  const [window, setWindow] = useState(() => guideWindow(Date.now() / 1000));
  const { from, to } = window;
  const [range, setRange] = useState(() => visibleRange(from, 150, 1000));
  const [rows, setRows] = useState<Record<string, Programme[]>>({});
  const [error, setError] = useState(false);
  const [loadedKey, setLoadedKey] = useState("");
  const [retry, setRetry] = useState(0);
  const [selected, setSelected] = useState<{
    p: Programme;
    ch: Channel;
  } | null>(null);
  const [day, setDay] = useState(() => localDay(now));
  const [jumpTime, setJumpTime] = useState("20:00");
  const [focus, setFocus] = useState<Focus>({
    channel: 0,
    time: now,
    channelButton: true,
  });
  const [pendingFocus, setPendingFocus] = useState<Focus | null>(null);
  const pendingScroll = useRef<number | null>(150);
  const width = ((to - from) / 60) * PX_PER_MIN;
  const x = (t: number) =>
    ((Math.min(Math.max(t, from), to) - from) / 60) * PX_PER_MIN;
  const virtualizer = useVirtualizer({
    count: channels.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW,
    overscan: 6,
  });
  const items = virtualizer.getVirtualItems();
  const first = items[0]?.index ?? 0;
  const last = items[items.length - 1]?.index ?? -1;
  const tabRow =
    focus.channel >= first && focus.channel <= last ? focus.channel : first;
  const ids = useMemo(
    () => [
      ...new Set(
        channels
          .slice(first, last + 1)
          .flatMap((ch) => (ch.epgId ? [ch.epgId] : [])),
      ),
    ],
    [channels, first, last],
  );

  const requestKey = `${sourceId}:${epgVersion}:${range.from}:${range.to}:${JSON.stringify(ids)}`;

  useEffect(() => {
    let cancelled = false;
    setError(false);
    if (!sourceId || !ids.length) return;
    const timer = globalThis.setTimeout(() => {
      api
        .guide(sourceId, ids, range.from, range.to)
        .then((result) => {
          if (!cancelled) {
            setRows(
              Object.fromEntries(
                result.map((row) => [row.epgId, row.programmes]),
              ),
            );
            setLoadedKey(requestKey);
          }
        })
        .catch(() => {
          if (!cancelled) setError(true);
        });
    }, 60);
    return () => {
      cancelled = true;
      globalThis.clearTimeout(timer);
    };
  }, [sourceId, ids, range.from, range.to, retry, requestKey]);

  function measure() {
    const el = scrollRef.current;
    if (!el) return;
    const next = visibleRange(from, el.scrollLeft, el.clientWidth);
    setRange((old) =>
      old.from === next.from && old.to === next.to ? old : next,
    );
    setDay(localDay(from + (el.scrollLeft / PX_PER_MIN) * 60 + HALF_HOUR));
    if (pendingScroll.current !== null) return;
    if (el.scrollLeft < 60) {
      pendingScroll.current = el.scrollLeft + (EXTEND / 60) * PX_PER_MIN;
      setWindow({ from: from - EXTEND, to });
    } else if (el.scrollLeft + el.clientWidth > CHANNEL_COL + width - 120) {
      setWindow({ from, to: to + EXTEND });
    }
  }

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (pendingScroll.current !== null) {
      el.scrollLeft = pendingScroll.current;
      pendingScroll.current = null;
    }
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [window]);

  function jump(time: number) {
    const next = guideWindow(time);
    pendingScroll.current = ((time - next.from - HALF_HOUR) / 60) * PX_PER_MIN;
    setWindow(next);
    setSelected(null);
  }

  function moveDay(delta: number) {
    const date = new Date((localTime(day, jumpTime) ?? now) * 1000);
    date.setDate(date.getDate() + delta);
    jump(date.getTime() / 1000);
  }

  function navigate(e: KeyboardEvent, current: Focus, programme?: Programme) {
    if (!e.key.startsWith("Arrow")) return;
    e.preventDefault();
    e.stopPropagation();
    const next = { ...current };
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      next.channel = Math.min(
        channels.length - 1,
        Math.max(0, current.channel + (e.key === "ArrowDown" ? 1 : -1)),
      );
    } else if (e.key === "ArrowLeft" && (current.channelButton || !programme)) {
      next.channelButton = true;
    } else {
      next.channelButton = false;
      if (programme)
        next.time =
          e.key === "ArrowRight" ? programme.stop : programme.start - 1;
      else if (!current.channelButton)
        next.time += e.key === "ArrowRight" ? HALF_HOUR : -HALF_HOUR;
    }
    virtualizer.scrollToIndex(next.channel, { align: "auto" });
    if (!next.channelButton) {
      const el = scrollRef.current;
      if (
        next.time < range.from + HALF_HOUR ||
        next.time >= range.to - HALF_HOUR
      )
        jump(next.time);
      else if (el) {
        const left = x(next.time);
        if (
          left < el.scrollLeft ||
          left > el.scrollLeft + el.clientWidth - CHANNEL_COL - 60
        )
          el.scrollLeft = Math.max(0, left - 60);
      }
    }
    setFocus(next);
    setPendingFocus(next);
  }

  useLayoutEffect(() => {
    if (!pendingFocus) return;
    if (!pendingFocus.channelButton && loadedKey !== requestKey && !error)
      return;
    const root = scrollRef.current;
    const ch = channels[pendingFocus.channel];
    if (!root || !ch) return;
    if (
      ch.epgId &&
      !(ch.epgId in rows) &&
      !pendingFocus.channelButton &&
      !error
    )
      return;
    const programme = ch.epgId
      ? programmeAt(rows[ch.epgId] ?? [], pendingFocus.time)
      : undefined;
    const buttons = root.querySelectorAll<HTMLButtonElement>(
      `[data-row="${pendingFocus.channel}"]`,
    );
    const button = Array.from(buttons).find((b) =>
      pendingFocus.channelButton || !programme
        ? b.dataset.time === undefined
        : Number(b.dataset.time) === programme.start,
    );
    if (button) {
      button.focus({ preventScroll: true });
      setPendingFocus(null);
    }
  }, [pendingFocus, rows, items, channels, error, loadedKey, requestKey]);

  const hours = useMemo(() => {
    const marks: number[] = [];
    for (
      let t = Math.max(from, range.from);
      t < Math.min(to, range.to);
      t += HALF_HOUR
    )
      marks.push(t);
    return marks;
  }, [from, to, range]);

  if (!channels.length)
    return <div className="empty">No channels to show in the guide.</div>;
  return (
    <div className="guide">
      <form
        className="guide-nav"
        onSubmit={(e) => {
          e.preventDefault();
          const time = localTime(day, jumpTime);
          if (time !== null) jump(time);
        }}
      >
        <button
          type="button"
          className="icon-btn"
          aria-label="Previous day"
          onClick={() => moveDay(-1)}
        >
          <ChevronLeft size={16} />
        </button>
        <input
          type="date"
          aria-label="Guide day"
          value={day}
          onChange={(e) => {
            setDay(e.target.value);
            const time = localTime(e.target.value, jumpTime);
            if (time !== null) jump(time);
          }}
          required
        />
        <button
          type="button"
          className="icon-btn"
          aria-label="Next day"
          onClick={() => moveDay(1)}
        >
          <ChevronRight size={16} />
        </button>
        <button
          type="button"
          className="btn"
          onClick={() => jump(Date.now() / 1000)}
        >
          Now
        </button>
        <input
          type="time"
          aria-label="Jump to time"
          value={jumpTime}
          onChange={(e) => setJumpTime(e.target.value)}
          required
        />
        <button className="btn" type="submit">
          Jump
        </button>
        <span className="guide-legend">
          <span className="sports">Sports</span>
          <span className="news">News</span>
          <span className="film">Film</span>
        </span>
      </form>
      {error && (
        <div className="guide-error" role="alert">
          Could not load programmes.{" "}
          <button className="btn" onClick={() => setRetry((n) => n + 1)}>
            Retry
          </button>
        </div>
      )}
      <div
        className="guide-scroll"
        ref={scrollRef}
        onScroll={measure}
        aria-label="TV guide. Use arrow keys to move between channels and programmes."
      >
        <div
          style={{
            width: CHANNEL_COL + width,
            height: virtualizer.getTotalSize() + 36,
            position: "relative",
          }}
        >
          <div className="guide-timebar" style={{ width: CHANNEL_COL + width }}>
            <div className="guide-corner" style={{ width: CHANNEL_COL }}>
              {fmtDay(localTime(day, "12:00") ?? now)}
            </div>
            {hours.map((t) => (
              <div
                key={t}
                className="guide-tick"
                style={{ left: CHANNEL_COL + x(t) }}
              >
                {fmtTime(t)}
                {new Date(t * 1000).getHours() === 0 &&
                  new Date(t * 1000).getMinutes() === 0 && (
                    <strong> · {fmtDay(t)}</strong>
                  )}
              </div>
            ))}
          </div>
          {now >= from && now < to && (
            <div
              className="guide-now"
              style={{
                left: CHANNEL_COL + x(now),
                height: virtualizer.getTotalSize(),
              }}
            />
          )}
          {items.map((row) => {
            const ch = channels[row.index];
            const progs = ch.epgId
              ? rows[ch.epgId]?.filter(
                  (p) =>
                    p.stop > Math.max(from, range.from) &&
                    p.start < Math.min(to, range.to),
                )
              : undefined;
            const focusedProgramme = programmeAt(progs ?? [], focus.time);
            return (
              <div
                key={ch.id}
                className="guide-row"
                style={{
                  transform: `translateY(${row.start + 36}px)`,
                  height: ROW,
                }}
              >
                <button
                  className={cx(
                    "guide-channel",
                    playing?.channel.id === ch.id && "playing",
                  )}
                  style={{ width: CHANNEL_COL }}
                  data-row={row.index}
                  tabIndex={
                    tabRow === row.index &&
                    (focus.channel !== tabRow ||
                      focus.channelButton ||
                      !focusedProgramme)
                      ? 0
                      : -1
                  }
                  onFocus={() =>
                    setFocus((old) => ({
                      ...old,
                      channel: row.index,
                      channelButton: true,
                    }))
                  }
                  onKeyDown={(e) =>
                    navigate(e, {
                      channel: row.index,
                      time: focus.time,
                      channelButton: true,
                    })
                  }
                  onClick={() => playChannel(ch, channels)}
                  title={`Watch ${ch.name}`}
                >
                  <Logo src={ch.logo} name={ch.name} />
                  <span>{splitPrefix(ch.name).rest}</span>
                </button>
                <div
                  className="guide-track"
                  style={{ left: CHANNEL_COL, width }}
                >
                  {(!ch.epgId || !progs?.length) && (
                    <div className="guide-none">
                      {!ch.epgId
                        ? "No guide data"
                        : !progs
                          ? error
                            ? "Guide unavailable"
                            : "Loading…"
                          : "No programmes in this time range"}
                    </div>
                  )}
                  {progs?.map((p) => {
                    const left = x(p.start),
                      w = Math.max(2, x(p.stop) - left - 2);
                    return (
                      <button
                        key={p.start}
                        data-row={row.index}
                        data-time={p.start}
                        className={cx(
                          "guide-prog",
                          `category-${programmeCategory(p.category)}`,
                          p.start <= now && p.stop > now && "live",
                          p.stop <= now && "past",
                        )}
                        style={{ left, width: w }}
                        tabIndex={
                          focus.channel === row.index &&
                          !focus.channelButton &&
                          focusedProgramme === p
                            ? 0
                            : -1
                        }
                        onFocus={() =>
                          setFocus((old) => ({
                            channel: row.index,
                            time:
                              p.start <= old.time && p.stop > old.time
                                ? old.time
                                : p.start,
                            channelButton: false,
                          }))
                        }
                        onKeyDown={(e) =>
                          navigate(
                            e,
                            {
                              channel: row.index,
                              time: focus.time,
                              channelButton: false,
                            },
                            p,
                          )
                        }
                        onClick={() => setSelected({ p, ch })}
                        onDoubleClick={() => playChannel(ch, channels)}
                        aria-label={`${p.title}, ${fmtRange(p)}${p.category ? `, ${p.category}` : ""}`}
                        title={`${p.title}\n${fmtRange(p)}${p.category ? `\n${p.category}` : ""}`}
                      >
                        <span className="gp-title">{p.title}</span>
                        {w > 90 && (
                          <span className="gp-time">{fmtRange(p)}</span>
                        )}
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
          <button
            className="icon-btn close"
            aria-label="Close programme details"
            onClick={() => setSelected(null)}
          >
            <X size={16} />
          </button>
          <div className="muted small">
            {splitPrefix(selected.ch.name).rest} · {fmtDay(selected.p.start)}{" "}
            {fmtRange(selected.p)}
            {selected.p.category && ` · ${selected.p.category}`}
          </div>
          <h3>{selected.p.title}</h3>
          {selected.p.subtitle && (
            <div className="np-sub">{selected.p.subtitle}</div>
          )}
          {selected.p.description && (
            <p className="np-desc">{selected.p.description}</p>
          )}
          <button
            className="btn primary"
            onClick={() => playChannel(selected.ch, channels)}
          >
            <Play size={15} /> Watch {splitPrefix(selected.ch.name).rest}
          </button>
        </div>
      )}
    </div>
  );
}
