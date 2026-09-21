import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { DayCount, Insights } from './api.ts';
import {
  addDays,
  buildHeatmap,
  compact,
  duration,
  monthChange,
  parseDay,
  prettifyApp,
  summaryText,
  toDayKey,
} from './stats.ts';

/**
 * The layout logic only — the totals it renders are computed in Rust and tested
 * there. What is easy to get wrong here is the week alignment: which column a
 * day lands in, and where the grid starts and ends.
 */

const WEEKS = 16;
const END = new Date(2026, 8, 17); // 17 Sep 2026, local midnight

function days(...entries: [string, number, number][]): DayCount[] {
  return entries.map(([day, words, dictations]) => ({ day, words, dictations }));
}

test('the grid is the requested number of whole weeks', () => {
  const grid = buildHeatmap([], WEEKS, END);
  assert.equal(grid.weeks.length, WEEKS);
  for (const column of grid.weeks) assert.equal(column.length, 7);
  assert.equal(grid.monthLabels.length, WEEKS);
});

test('columns run Sunday to Saturday, with no gaps or repeats', () => {
  const grid = buildHeatmap([], WEEKS, END);
  const flat = grid.weeks.flat();
  assert.equal(flat[0].date.getDay(), 0, 'first cell is a Sunday');

  const seen = new Set<string>();
  for (let i = 0; i < flat.length; i += 1) {
    assert.equal(flat[i].date.getDay(), i % 7, `cell ${i} sits on the right weekday`);
    if (i > 0) {
      assert.equal(
        flat[i].date.getTime() - flat[i - 1].date.getTime(),
        86_400_000,
        `cell ${i} follows the previous day`,
      );
    }
    assert.ok(!seen.has(flat[i].key), `day ${flat[i].key} appears once`);
    seen.add(flat[i].key);
  }
});

test('the last cell is the Saturday ending the week that contains today', () => {
  const grid = buildHeatmap([], WEEKS, END);
  const last = grid.weeks[WEEKS - 1][6];

  assert.equal(last.date.getDay(), 6);
  const gap = (last.date.getTime() - END.getTime()) / 86_400_000;
  assert.ok(gap >= 0 && gap < 7, `expected the week to end within 7 days, gap was ${gap}`);
});

test('a day lands in the cell that matches its date', () => {
  const grid = buildHeatmap([], WEEKS, END);
  const target = grid.weeks[3][2];

  const filled = buildHeatmap(days([toDayKey(target.date), 500, 4]), WEEKS, END);

  assert.equal(filled.weeks[3][2].words, 500);
  assert.equal(filled.weeks[3][2].dictations, 4);
  // Every other cell stays empty, so nothing was written twice.
  assert.equal(
    filled.weeks.flat().filter((c) => c.dictations > 0).length,
    1,
  );
});

test('days outside the window are ignored rather than shifted', () => {
  const grid = buildHeatmap([], WEEKS, END);
  const oldest = grid.weeks[0][0].date;
  const before = toDayKey(addDays(oldest, -1));
  const after = toDayKey(addDays(grid.weeks[WEEKS - 1][6].date, 1));

  const filled = buildHeatmap(days([before, 900, 1], [after, 900, 1]), WEEKS, END);
  assert.equal(filled.weeks.flat().filter((c) => c.dictations > 0).length, 0);
});

test('intensity is relative to the busiest day in view', () => {
  const grid = buildHeatmap([], WEEKS, END);
  const busiest = grid.weeks[5][1].date;
  const half = toDayKey(addDays(busiest, 1));
  const sliver = toDayKey(addDays(busiest, 2));

  const filled = buildHeatmap(
    days([toDayKey(busiest), 1_000, 1], [half, 500, 1], [sliver, 100, 1]),
    WEEKS,
    END,
  );

  assert.equal(filled.weeks[5][1].level, 4, 'the busiest day is the darkest');
  assert.equal(filled.weeks[5][2].level, 3, 'half the busiest');
  assert.equal(filled.weeks[5][3].level, 1, 'a tenth is the palest, not empty');
  assert.equal(filled.weeks[5][4].level, 0, 'a day with no dictation is empty');
});

test('a quiet history still shows a gradient', () => {
  // With one dictation of 3 words, a fixed threshold would paint everything the
  // palest shade and the heatmap would read as flat.
  const grid = buildHeatmap([], WEEKS, END);
  const only = grid.weeks[2][3].date;
  const filled = buildHeatmap(days([toDayKey(only), 3, 1]), WEEKS, END);
  assert.equal(filled.weeks[2][3].level, 4);
});

test('month labels appear only where the month changes', () => {
  const grid = buildHeatmap([], WEEKS, END);
  assert.notEqual(grid.monthLabels[0], '', 'the first column is always labelled');

  for (let w = 1; w < WEEKS; w += 1) {
    const sameMonth =
      grid.weeks[w][0].date.getMonth() === grid.weeks[w - 1][0].date.getMonth();
    assert.equal(
      grid.monthLabels[w] === '',
      sameMonth,
      `column ${w} label should be blank exactly when the month is unchanged`,
    );
  }
});

/* ------------------------------------------------------------- helpers */

test('app classes are readable but recognisable', () => {
  assert.equal(prettifyApp('google-chrome-beta'), 'Google Chrome Beta');
  assert.equal(prettifyApp('com.mitchellh.ghostty'), 'Ghostty');
  assert.equal(prettifyApp('org.wezfurlong.wezterm'), 'Wezterm');
  assert.equal(prettifyApp('kitty'), 'Kitty');
  assert.equal(prettifyApp(''), 'Unknown');
  // Dictating with the settings window focused is real but is not a target app.
  assert.equal(prettifyApp('orra'), 'Orra (its own window)');
});

test('days parse as local dates, not UTC', () => {
  // `new Date('2026-09-17')` is parsed as UTC midnight, which lands on the 16th
  // for anyone west of Greenwich — this is the bug the helper exists to avoid.
  const d = parseDay('2026-09-17');
  assert.equal(d.getFullYear(), 2026);
  assert.equal(d.getMonth(), 8);
  assert.equal(d.getDate(), 17);
  assert.equal(toDayKey(d), '2026-09-17');
});

test('month change has no answer without a previous month', () => {
  const base = { words_this_month: 100, words_last_month: 0 } as Insights['totals'];
  assert.equal(monthChange(base), null, 'nothing to compare against');

  const down = { words_this_month: 50, words_last_month: 100 } as Insights['totals'];
  assert.equal(monthChange(down), -50);

  const up = { words_this_month: 150, words_last_month: 100 } as Insights['totals'];
  assert.equal(monthChange(up), 50);
});

test('numbers are compacted only when that reads better', () => {
  assert.equal(compact(0), '0');
  assert.equal(compact(812), '812');
  assert.equal(compact(9_999), '9,999');
  assert.equal(compact(34_000), '34K');
  assert.equal(compact(1_166_378), '1.2M');
});

test('durations read as time spent, not milliseconds', () => {
  assert.equal(duration(4_200), '4.2s');
  assert.equal(duration(45_000), '45s');
  assert.equal(duration(75_000), '1m 15s');
  assert.equal(duration(120_000), '2m');
});

test('the copied summary carries the real figures', () => {
  const insights = {
    totals: {
      words: 219,
      dictations: 8,
      apps: 2,
      wpm: 141.4,
      measured: 3,
      fixes: 4,
      fillers: 6,
      confidence: 0.96,
      vocabulary: 120,
      words_this_month: 219,
      words_last_month: 0,
      minutes_saved: 5,
      books: 0.002,
    },
    days: [],
    apps: [],
    current_streak: 2,
    longest_streak: 5,
  } as Insights;

  const text = summaryText(insights);
  assert.match(text, /219 words dictated across 8 dictations/);
  assert.match(text, /141 words per minute/);
  assert.match(text, /2 day streak \(longest 5\)/);
  assert.match(text, /120 distinct words/);
  assert.match(text, /4 replacements and 6 fillers/);
  // Nothing invented: no figure that was not supplied.
  assert.doesNotMatch(text, /1,166,378/);
});

test('a summary with no measured rate omits the rate line', () => {
  const insights = {
    totals: { words: 0, dictations: 0, wpm: null, fixes: 0, fillers: 0, vocabulary: 0 },
    days: [],
    apps: [],
    current_streak: 0,
    longest_streak: 0,
  } as unknown as Insights;

  const text = summaryText(insights);
  assert.doesNotMatch(text, /words per minute/);
  assert.doesNotMatch(text, /replacements/);
});
