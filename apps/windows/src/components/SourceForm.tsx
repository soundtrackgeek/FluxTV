import { useState } from "react";
import { Loader2 } from "lucide-react";
import { api, errorMessage, type Source, type SourceKind } from "../api";
import { cx } from "../util";

/** Add (or edit) an IPTV provider. Xtream credentials are validated before saving. */
export function SourceForm({ existing, onSaved, onCancel }: { existing?: Source; onSaved: (s: Source) => void; onCancel?: () => void }) {
  const [kind, setKind] = useState<SourceKind>(existing?.kind ?? "xtream");
  const [name, setName] = useState(existing?.name ?? "");
  const [url, setUrl] = useState(existing?.url ?? "");
  const [username, setUsername] = useState(existing?.username ?? "");
  const [password, setPassword] = useState("");
  const [epgUrl, setEpgUrl] = useState(existing?.epgUrl ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const input = {
      name,
      kind,
      url,
      username: kind === "xtream" ? username : null,
      password: kind === "xtream" ? password || null : null,
      epgUrl: epgUrl || null,
    };
    try {
      const saved = existing ? await api.updateSource(existing.id, input) : await api.addSource(input);
      onSaved(saved);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form className="source-form" onSubmit={submit}>
      <div className="segmented">
        <button type="button" className={cx(kind === "xtream" && "on")} onClick={() => setKind("xtream")}>
          Xtream Codes
        </button>
        <button type="button" className={cx(kind === "m3u" && "on")} onClick={() => setKind("m3u")}>
          M3U playlist
        </button>
      </div>

      {kind === "xtream" ? (
        <>
          <label>
            Server URL
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="http://provider.example:8080" required spellCheck={false} />
          </label>
          <div className="row gap">
            <label className="grow">
              Username
              <input value={username} onChange={(e) => setUsername(e.target.value)} required autoComplete="off" spellCheck={false} />
            </label>
            <label className="grow">
              Password
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required={!existing}
                placeholder={existing ? "unchanged" : ""}
                autoComplete="new-password"
              />
            </label>
          </div>
          <p className="hint">Tip: you can paste a full <code>get.php?username=…&amp;password=…</code> link into the server field.</p>
        </>
      ) : (
        <>
          <label>
            Playlist URL
            <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/playlist.m3u" required spellCheck={false} />
          </label>
          <p className="hint">Xtream links (<code>get.php?…</code>) are detected automatically and use the faster Xtream API.</p>
        </>
      )}

      <label>
        <span>
          TV guide (XMLTV) URL <span className="muted">— optional</span>
        </span>
        <input
          value={epgUrl}
          onChange={(e) => setEpgUrl(e.target.value)}
          placeholder={kind === "xtream" ? "Uses the provider's xmltv.php by default" : "Uses the playlist's url-tvg if present"}
          spellCheck={false}
        />
      </label>
      <label>
        <span>
          Name <span className="muted">— optional</span>
        </span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="My IPTV" />
      </label>

      {error && <div className="form-error">{error}</div>}
      <div className="row gap end">
        {onCancel && (
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button type="submit" className="btn primary" disabled={busy}>
          {busy && <Loader2 size={15} className="spin" />} {existing ? "Save" : busy ? "Connecting…" : "Add provider"}
        </button>
      </div>
    </form>
  );
}
