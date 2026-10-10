import assert from "node:assert/strict";
import test from "node:test";
import {
  guideWindow,
  localDay,
  localTime,
  programmeAt,
  programmeCategory,
  visibleRange,
} from "../src/guide.ts";
import type { Programme } from "../src/api.ts";

const programme = (start: number, stop: number): Programme => ({
  channel: "test",
  start,
  stop,
  title: "Test",
  subtitle: null,
  description: null,
  category: null,
  icon: null,
  episode: null,
});

test("queries follow only the viewport, with half-hour overscan and stable buckets", () => {
  assert.deepEqual(visibleRange(36000, 151, 1110), { from: 36000, to: 52200 });
  assert.deepEqual(
    visibleRange(36000, 152, 1110),
    visibleRange(36000, 151, 1110),
  );
  assert.deepEqual(visibleRange(36000, 1950, 1110), { from: 57600, to: 72000 });
});

test("day and time jumps use local calendar dates across daylight-saving changes", () => {
  process.env.TZ = "Europe/Oslo";
  const before = localTime("2026-10-24", "20:15")!;
  const after = localTime("2026-10-25", "20:15")!;
  assert.equal(after - before, 25 * 3600);
  assert.equal(localDay(after), "2026-10-25");
  assert.equal(new Date(after * 1000).getHours(), 20);
  assert.equal(localTime("", ""), null);
  const window = guideWindow(after);
  assert.ok(window.from < after && window.to > after);
});

test("vertical navigation preserves time across unequal schedules and gaps", () => {
  const rows = [programme(100, 200), programme(200, 600), programme(800, 900)];
  assert.equal(programmeAt(rows, 250), rows[1]);
  assert.equal(programmeAt(rows, 200), rows[1]);
  assert.equal(programmeAt(rows, 740), rows[2]);
  assert.equal(programmeAt([], 250), undefined);
});

test("category colors recognize provider labels and leave other genres neutral", () => {
  assert.equal(programmeCategory("Live SPORTS / Football"), "sports");
  assert.equal(programmeCategory("Nyheter"), "news");
  assert.equal(programmeCategory("Movie / Drama"), "film");
  assert.equal(programmeCategory("Children"), "other");
  assert.equal(programmeCategory(null), "other");
});
