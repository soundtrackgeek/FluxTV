import { useEffect, useState } from "react";
import { Tv } from "lucide-react";
import { api, events } from "./api";
import { useApp, useEpg } from "./store";
import { cx } from "./util";
import { Browse } from "./components/Browse";
import { NowPlaying } from "./components/NowPlaying";
import { Player } from "./components/Player";
import { Settings } from "./components/Settings";
import { Sidebar } from "./components/Sidebar";
import { SourceForm } from "./components/SourceForm";

function Welcome() {
  const { loadSources, setSource } = useApp.getState();
  return (
    <div className="welcome">
      <div className="welcome-card">
        <div className="brand big">
          <div className="brand-mark">
            <Tv size={26} />
          </div>
          <span>FluxTV</span>
        </div>
        <p className="muted">Add your IPTV provider to get started. Your credentials are protected by Windows Credential Manager.</p>
        <SourceForm
          onSaved={async (s) => {
            await loadSources();
            setSource(s.id);
            api.refreshSource(s.id, true);
          }}
        />
      </div>
    </div>
  );
}

export default function App() {
  const { sources, sourceId, screen, display, playing, fullscreen, toast, kind, query } = useApp();
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const s = useApp.getState();
    s.loadSources().then((list) => {
      setReady(true);
      const id = useApp.getState().sourceId;
      // Refresh stale catalog/guide in the background on startup.
      if (id && list.length) api.refreshSource(id, false).catch(() => {});
    });

    const unlisten = [
      events.onProgress((p) => useApp.getState().setProgress(p)),
      events.onError((e) => {
        const st = useApp.getState();
        st.setProgress({ sourceId: e.sourceId, stage: e.stage as "catalog" | "epg", message: e.message, fraction: null, done: true });
        st.showToast(`${e.stage === "epg" ? "TV guide" : "Channel list"} refresh failed: ${e.message}`, "error");
      }),
      events.onCatalogUpdated(async (id) => {
        const st = useApp.getState();
        await st.loadSources();
        if (id === st.sourceId) st.bumpCatalog();
      }),
      events.onEpgUpdated(async (id) => {
        const st = useApp.getState();
        await st.loadSources();
        if (id === st.sourceId) {
          useEpg.getState().clear();
          st.bumpEpg();
        }
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((f) => f()));
    };
  }, []);

  // Changing provider clears cached guide data.
  useEffect(() => useEpg.getState().clear(), [sourceId]);

  if (!ready) return <div className="boot" />;
  if (sources.length === 0) return <Welcome />;

  const guideMode = screen === "browse" && display === "guide" && kind === "live" && !query.trim();

  return (
    <div className={cx("app", guideMode && "guide-mode", screen === "settings" && "settings-mode", fullscreen && "is-fullscreen")}>
      <Sidebar />
      <main className="main">{screen === "settings" ? <Settings /> : <Browse />}</main>
      <aside className={cx("stage", playing && "has-native-video", !playing && "idle", (guideMode || screen === "settings") && playing && "mini")}>
        <Player />
        {!guideMode && screen === "browse" && <NowPlaying />}
      </aside>
      {toast && <div className={cx("toast", toast.tone)}>{toast.message}</div>}
    </div>
  );
}
