import { useEffect, useState } from "react";
import { Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api, errorMessage, type AccountInfo, type ExternalPlayer, type Source } from "../api";
import { stopPlayback } from "../actions";
import { useApp } from "../store";
import { fmtAgo } from "../util";
import { SourceForm } from "./SourceForm";

function SourceCard({ source, onEdit }: { source: Source; onEdit: () => void }) {
  const { sourceId, progress } = useApp();
  const { setSource, loadSources, showToast } = useApp.getState();
  const [account, setAccount] = useState<AccountInfo | null | undefined>(undefined);
  const [accountError, setAccountError] = useState<string | null>(null);
  const busy = ["catalog", "epg"].some((s) => {
    const p = progress[`${source.id}:${s}`];
    return p && !p.done;
  });

  useEffect(() => {
    if (source.kind !== "xtream") return;
    api
      .accountInfo(source.id)
      .then(setAccount)
      .catch((e) => setAccountError(errorMessage(e)));
  }, [source.id, source.kind]);

  const remove = async () => {
    if (!window.confirm(`Remove “${source.name}” and its cached channels, guide, favorites and history?`)) return;
    if (sourceId === source.id) await stopPlayback();
    await api.deleteSource(source.id);
    const rest = await loadSources();
    if (sourceId === source.id) setSource(rest[0]?.id ?? null);
    showToast(`Removed ${source.name}`);
  };

  return (
    <div className="card source-card">
      <div className="row between">
        <div>
          <div className="card-title">
            {source.name} {sourceId === source.id && <span className="pill accent">Active</span>}
          </div>
          <div className="muted small">
            {source.kind === "xtream" ? "Xtream Codes" : "M3U"} · {source.url}
            {source.username && ` · ${source.username}`}
          </div>
        </div>
        <div className="row gap">
          {sourceId !== source.id && (
            <button className="btn" onClick={() => setSource(source.id)}>
              Use
            </button>
          )}
          <button
            className="btn"
            disabled={busy}
            onClick={() => api.refreshSource(source.id, true).catch((e) => showToast(errorMessage(e), "error"))}
          >
            <RefreshCw size={14} className={busy ? "spin" : ""} /> Refresh
          </button>
          <button className="icon-btn" title="Edit" onClick={onEdit}>
            <Pencil size={15} />
          </button>
          <button className="icon-btn danger" title="Remove" onClick={remove}>
            <Trash2 size={15} />
          </button>
        </div>
      </div>
      <dl className="facts">
        <dt>Channels updated</dt>
        <dd>{fmtAgo(source.catalogUpdatedAt)}</dd>
        <dt>Guide updated</dt>
        <dd>{fmtAgo(source.epgUpdatedAt)}</dd>
        {account && (
          <>
            <dt>Subscription</dt>
            <dd>
              {account.status}
              {account.isTrial && " (trial)"}
              {account.expiresAt && ` · expires ${new Date(account.expiresAt * 1000).toLocaleDateString()}`}
            </dd>
            <dt>Connections</dt>
            <dd>
              {account.activeConnections} of {account.maxConnections} in use
            </dd>
            <dt>Stream formats</dt>
            <dd>{account.allowedFormats.join(", ") || "—"}</dd>
          </>
        )}
        {accountError && (
          <>
            <dt>Account</dt>
            <dd className="error-text">{accountError}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

export function Settings() {
  const { sources } = useApp();
  const { loadSources, setSource, showToast } = useApp.getState();
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Source | null>(null);
  const [player, setPlayer] = useState<ExternalPlayer | null>(null);
  const [playerPath, setPlayerPath] = useState("");
  const [version, setVersion] = useState("");

  useEffect(() => {
    api.externalPlayer().then(setPlayer);
    api.getSetting("external_player").then((p) => setPlayerPath(p ?? ""));
    api.appInfo().then((i) => setVersion(i.version));
  }, []);

  const savePlayer = async () => {
    await api.setSetting("external_player", playerPath.trim() || null);
    const p = await api.externalPlayer();
    setPlayer(p);
    showToast(p ? `External player: ${p.name}` : "No external player found", p ? "info" : "error");
  };

  return (
    <div className="settings">
      <h1>Settings</h1>

      <section>
        <div className="row between">
          <h2>Providers</h2>
          {!adding && (
            <button className="btn" onClick={() => setAdding(true)}>
              <Plus size={15} /> Add provider
            </button>
          )}
        </div>
        {adding && (
          <div className="card">
            <SourceForm
              onCancel={() => setAdding(false)}
              onSaved={async (s) => {
                setAdding(false);
                await loadSources();
                setSource(s.id);
                api.refreshSource(s.id, true);
              }}
            />
          </div>
        )}
        {sources.map((s) =>
          editing?.id === s.id ? (
            <div className="card" key={s.id}>
              <SourceForm
                existing={s}
                onCancel={() => setEditing(null)}
                onSaved={async () => {
                  setEditing(null);
                  await loadSources();
                  api.refreshSource(s.id, true);
                }}
              />
            </div>
          ) : (
            <SourceCard key={s.id} source={s} onEdit={() => setEditing(s)} />
          ),
        )}
      </section>

      <section>
        <h2>External player</h2>
        <p className="muted">
          Used for streams the built-in player can't decode (unusual codecs or containers). mpv and VLC are detected
          automatically.
        </p>
        <div className="card">
          <div className="small">
            {player ? (
              <>
                Using <strong>{player.name}</strong> — <code>{player.path}</code>
              </>
            ) : (
              "No external player found."
            )}
          </div>
          <div className="row gap" style={{ marginTop: 10 }}>
            <input
              className="grow"
              value={playerPath}
              onChange={(e) => setPlayerPath(e.target.value)}
              placeholder="Custom path, e.g. C:\Program Files\mpv\mpv.exe"
              spellCheck={false}
            />
            <button className="btn" onClick={savePlayer}>
              Save
            </button>
          </div>
        </div>
      </section>

      <section>
        <h2>Keyboard shortcuts</h2>
        <div className="card shortcuts">
          <kbd>F</kbd> <span>Toggle fullscreen (or double-click the video)</span>
          <kbd>Esc</kbd> <span>Exit fullscreen</span>
          <kbd>Space</kbd> <span>Pause / resume</span>
          <kbd>M</kbd> <span>Mute</span>
          <kbd>PgUp / PgDn</kbd> <span>Previous / next channel (↑ / ↓ in fullscreen)</span>
          <kbd>Ctrl + K</kbd> <span>Search</span>
        </div>
      </section>

      <section>
        <h2>History</h2>
        <button
          className="btn"
          onClick={async () => {
            const id = useApp.getState().sourceId;
            if (id) {
              await api.clearHistory(id).catch((e) => showToast(errorMessage(e), "error"));
              showToast("Watch history cleared");
            }
          }}
        >
          Clear watch history
        </button>
      </section>

      <p className="muted small">FluxTV {version}</p>
    </div>
  );
}
