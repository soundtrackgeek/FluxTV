import { RefreshCw } from "lucide-react";
import { useApp } from "../store";

/** Compact progress indicator for background catalog/guide refreshes. */
export function RefreshStatus() {
  const { progress, sourceId } = useApp();
  const active = Object.values(progress).filter((p) => p.sourceId === sourceId && !p.done);
  if (active.length === 0) return null;
  const p = active[active.length - 1];
  return (
    <div className="refresh-status" title={p.message}>
      <div className="refresh-line">
        <RefreshCw size={13} className="spin" />
        <span>{p.message}</span>
      </div>
      <div className="bar">
        <div className={p.fraction == null ? "indeterminate" : ""} style={{ width: `${Math.round((p.fraction ?? 0.3) * 100)}%` }} />
      </div>
    </div>
  );
}
