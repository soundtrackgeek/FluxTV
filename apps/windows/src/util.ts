import { useEffect, useState } from "react";
import type { Programme } from "./api";

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short" });

export const fmtTime = (unix: number) => timeFmt.format(new Date(unix * 1000));
export const fmtDay = (unix: number) => dayFmt.format(new Date(unix * 1000));
export const fmtRange = (p: Programme) => `${fmtTime(p.start)} – ${fmtTime(p.stop)}`;

export function fmtAgo(unix: number | null): string {
  if (!unix) return "never";
  const s = Math.max(0, Date.now() / 1000 - unix);
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400 * 2) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

export function progressOf(p: Programme, nowSec: number): number {
  return Math.min(1, Math.max(0, (nowSec - p.start) / Math.max(1, p.stop - p.start)));
}

export function minutesLeft(p: Programme, nowSec: number): number {
  return Math.max(0, Math.round((p.stop - nowSec) / 60));
}

/** Current Unix time in seconds, re-rendering every `intervalMs`. */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now() / 1000), intervalMs);
    return () => window.clearInterval(t);
  }, [intervalMs]);
  return now;
}

export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = window.setTimeout(() => setV(value), ms);
    return () => window.clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** Strip the `XX| ` style prefix providers put in front of channel/category names. */
export function splitPrefix(name: string): { prefix: string | null; rest: string } {
  const m = /^\s*([^|:]{1,10})\s*[|:]\s*(.+)$/.exec(name);
  if (m && !/\d{2}:\d{2}/.test(name.slice(0, 8))) return { prefix: m[1].trim(), rest: m[2].trim() };
  return { prefix: null, rest: name.trim() };
}

export function initials(name: string): string {
  const { rest } = splitPrefix(name);
  const words = rest.replace(/[^\p{L}\p{N} ]/gu, " ").split(/\s+/).filter(Boolean);
  return (words[0]?.[0] ?? "?").toUpperCase() + (words[1]?.[0] ?? "").toUpperCase();
}

export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(" ");
}
