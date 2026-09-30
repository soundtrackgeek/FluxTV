import { useEffect, useRef } from "react";
import { CalendarDays, List, Search, X } from "lucide-react";
import { selectionTitle, useChannels } from "../actions";
import { useApp } from "../store";
import { cx } from "../util";
import { ChannelList } from "./ChannelList";
import { Guide } from "./Guide";

export function Browse() {
  const { query, display, kind, selection } = useApp();
  const { setQuery, setDisplay } = useApp.getState();
  const { channels, loading, searching } = useChannels();
  const searchRef = useRef<HTMLInputElement>(null);

  // Ctrl+K or "/" focuses search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const inInput = (e.target as HTMLElement).tagName === "INPUT";
      if ((e.ctrlKey && e.key.toLowerCase() === "k") || (e.key === "/" && !inInput)) {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const emptyHint = searching
    ? "No matches. Try fewer words."
    : selection.type === "favorites"
      ? "No favorites yet. Click the ☆ next to a channel to add it here."
      : selection.type === "recent"
        ? "Nothing watched yet."
        : loading
          ? ""
          : "This category is empty.";

  const showGuide = display === "guide" && kind === "live" && !searching;

  return (
    <section className={cx("browse", showGuide && "with-guide")}>
      <header className="browse-header">
        <div className="search">
          <Search size={16} />
          <input
            ref={searchRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && setQuery("")}
            placeholder={kind === "live" ? "Search 50,000+ channels  (Ctrl+K)" : "Search movies  (Ctrl+K)"}
            spellCheck={false}
          />
          {query && (
            <button className="icon-btn tiny" onClick={() => setQuery("")} title="Clear search">
              <X size={14} />
            </button>
          )}
        </div>
        <div className="row between">
          <h2 className="list-title" title={selectionTitle()}>
            {selectionTitle()}
            <span className="muted"> {loading ? "…" : channels.length}</span>
          </h2>
          {kind === "live" && !searching && (
            <div className="segmented small">
              <button className={cx(display === "list" && "on")} onClick={() => setDisplay("list")} title="List">
                <List size={14} /> List
              </button>
              <button className={cx(display === "guide" && "on")} onClick={() => setDisplay("guide")} title="TV guide">
                <CalendarDays size={14} /> Guide
              </button>
            </div>
          )}
        </div>
      </header>
      {showGuide ? <Guide channels={channels} /> : <ChannelList channels={channels} loading={loading} emptyHint={emptyHint} />}
    </section>
  );
}
