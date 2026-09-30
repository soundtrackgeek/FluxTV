import { useState } from "react";
import { initials } from "../util";

/** Channel logo with a lettered fallback (many provider logos are dead links). */
export function Logo({ src, name, className }: { src: string | null; name: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  if (!src || failed) {
    let hash = 0;
    for (const ch of name) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
    return (
      <div className={`logo logo-fallback ${className ?? ""}`} style={{ "--hue": Math.abs(hash) % 360 } as React.CSSProperties}>
        {initials(name)}
      </div>
    );
  }
  return (
    <div className={`logo ${className ?? ""}`}>
      <img src={src} alt="" loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setFailed(true)} />
    </div>
  );
}
