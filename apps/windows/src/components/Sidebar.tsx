import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Clock, Film, Search, Settings, Star, Tv, X } from "lucide-react";
import { api, type Category } from "../api";
import { useApp } from "../store";
import { cx, splitPrefix } from "../util";
import { RefreshStatus } from "./RefreshStatus";

export function Sidebar() {
  const { sourceId, kind, selection, screen, catalogVersion, sources } = useApp();
  const { setKind, select, setScreen, setSource } = useApp.getState();
  const [categories, setCategories] = useState<Category[]>([]);
  const [filter, setFilter] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!sourceId) return;
    let cancelled = false;
    api.categories(sourceId, kind).then((c) => !cancelled && setCategories(c.filter((x) => x.channelCount > 0)));
    return () => {
      cancelled = true;
    };
  }, [sourceId, kind, catalogVersion]);

  const visible = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (!f) return categories;
    const words = f.split(/\s+/);
    return categories.filter((c) => words.every((w) => c.name.toLowerCase().includes(w)));
  }, [categories, filter]);

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 34,
    overscan: 12,
  });

  const isBrowse = screen === "browse";
  const active = (t: string, id?: string) =>
    isBrowse && selection.type === t && (id === undefined || (selection.type === "category" && selection.id === id));

  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark">
          <Tv size={18} />
        </div>
        <span>FluxTV</span>
        {sources.length > 1 && (
          <select
            className="source-picker"
            value={sourceId ?? ""}
            onChange={(e) => setSource(Number(e.target.value))}
            title="Switch provider"
          >
            {sources.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        )}
      </div>

      <div className="segmented">
        <button className={cx(kind === "live" && "on")} onClick={() => setKind("live")}>
          <Tv size={15} /> Live TV
        </button>
        <button className={cx(kind === "movie" && "on")} onClick={() => setKind("movie")}>
          <Film size={15} /> Movies
        </button>
      </div>

      <nav className="nav">
        <button className={cx("nav-item", active("favorites") && "on")} onClick={() => select({ type: "favorites" })}>
          <Star size={16} /> Favorites
        </button>
        <button className={cx("nav-item", active("recent") && "on")} onClick={() => select({ type: "recent" })}>
          <Clock size={16} /> Recently watched
        </button>
      </nav>

      <div className="section-title">
        Categories <span className="muted">{categories.length}</span>
      </div>
      <div className="filter">
        <Search size={14} />
        <input placeholder="Filter categories" value={filter} onChange={(e) => setFilter(e.target.value)} spellCheck={false} />
        {filter && (
          <button className="icon-btn tiny" onClick={() => setFilter("")} title="Clear">
            <X size={13} />
          </button>
        )}
      </div>
      <div className="category-scroll" ref={scrollRef}>
        {categories.length === 0 && <div className="empty small">No categories yet</div>}
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((row) => {
            const c = visible[row.index];
            const { prefix, rest } = splitPrefix(c.name);
            return (
              <button
                key={c.id}
                className={cx("category", active("category", c.id) && "on")}
                style={{ transform: `translateY(${row.start}px)`, height: row.size }}
                onClick={() => select({ type: "category", id: c.id, name: c.name })}
                title={c.name}
              >
                {prefix && <span className="prefix">{prefix}</span>}
                <span className="name">{rest}</span>
                <span className="count">{c.channelCount}</span>
              </button>
            );
          })}
        </div>
      </div>

      <RefreshStatus />
      <button className={cx("nav-item settings-link", screen === "settings" && "on")} onClick={() => setScreen("settings")}>
        <Settings size={16} /> Settings
      </button>
    </aside>
  );
}
