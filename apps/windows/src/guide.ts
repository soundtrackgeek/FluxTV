import type { Programme } from "./api";

export const HALF_HOUR = 1800;
export const EXTEND = 6 * 3600;
export const PX_PER_MIN = 5;
export const CHANNEL_COL = 210;

export function guideWindow(time: number) {
  const start = Math.floor(time / HALF_HOUR) * HALF_HOUR - 3600;
  return { from: start, to: start + EXTEND };
}

/** Half-hour buckets prevent a database request on every scroll pixel. */
export function visibleRange(from: number, left: number, width: number) {
  const start = from + (Math.max(0, left) / PX_PER_MIN) * 60;
  const end = start + (Math.max(60, width - CHANNEL_COL) / PX_PER_MIN) * 60;
  return {
    from: Math.floor(start / HALF_HOUR) * HALF_HOUR - HALF_HOUR,
    to: Math.ceil(end / HALF_HOUR) * HALF_HOUR + HALF_HOUR,
  };
}

export function localDay(time: number) {
  const date = new Date(time * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

/** Use local calendar components, including days with a daylight-saving change. */
export function localTime(day: string, time: string): number | null {
  const [year, month, date] = day.split("-").map(Number);
  const [hour, minute] = time.split(":").map(Number);
  if (![year, month, date, hour, minute].every(Number.isFinite)) return null;
  const value = new Date(year, month - 1, date, hour, minute).getTime() / 1000;
  return Number.isFinite(value) ? value : null;
}

export function programmeCategory(
  category: string | null,
): "sports" | "news" | "film" | "other" {
  const value = category?.toLocaleLowerCase() ?? "";
  if (/sport|football|soccer|tennis|basketball/.test(value)) return "sports";
  if (/news|current affairs|nyhet/.test(value)) return "news";
  if (/film|movie|cinema/.test(value)) return "film";
  return "other";
}

/** Preserve the intended time when moving vertically through unequal programme lengths. */
export function programmeAt(programmes: Programme[], time: number) {
  return (
    programmes.find((p) => p.start <= time && p.stop > time) ??
    programmes.reduce<Programme | undefined>(
      (best, p) =>
        !best || Math.abs(p.start - time) < Math.abs(best.start - time)
          ? p
          : best,
      undefined,
    )
  );
}
