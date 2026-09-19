/**
 * Presentation helpers for the Insights page.
 *
 * The arithmetic lives in Rust (`src-tauri/src/stats.rs`) so it is covered by
 * `cargo test`; everything here is layout and wording, plus the local-date
 * parsing that the browser is the right tool for.
 */

import type { DayCount, Insights } from './api';

/* ------------------------------------------------------------ formatting */

/** `1166378` → `1,166,378`. */
export function num(value: number): string {
  return Math.round(value).toLocaleString();
}

/**
 * Compact form for stat tiles, e.g. `1.2M`, `34K`, `812`.
 *
 * Only shortens where the shorter form is still informative: 9,999 stays exact,
 * because rounding it to "10K" both loses precision and overstates the figure.
 */
export function compact(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (value >= 10_000) return `${Math.round(value / 1_000)}K`;
  return num(value);
}

/** Relative time: `just now`, `5m ago`, `3h ago`, then a date and time. */
export function timeAgo(ms: number): string {
  const secs = Math.max(0, (Date.now() - ms) / 1000);
  if (secs < 60) return 'just now';
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  const d = new Date(ms);
  return (
    d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) +
    ' ' +
    d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  );
}

/** `4_200` → `4.2s`, `75_000` → `1m 15s`. */
export function duration(ms: number): string {
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(secs < 10 ? 1 : 0)}s`;
  const mins = Math.floor(secs / 60);
  const rest = Math.round(secs % 60);
  return rest === 0 ? `${mins}m` : `${mins}m ${rest}s`;
}

/**
 * A window class is what the compositor reports — `google-chrome-beta`,
 * `com.mitchellh.ghostty`, `org.wezfurlong.wezterm` — so it needs tidying before
 * it can be read as an application name.
 */
export function prettifyApp(appClass: string): string {
  if (!appClass) return 'Unknown';
  if (appClass === 'orra') return 'Orra (its own window)';

  // Drop a reverse-DNS prefix: com.mitchellh.ghostty -> ghostty.
  const parts = appClass.split('.');
  const tail = parts.length > 1 ? parts[parts.length - 1] : appClass;

  return tail
    .split(/[-_ ]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/* --------------------------------------------------------------- dates */

/** Parses `YYYY-MM-DD` as a **local** day, never UTC. */
export function parseDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y, m - 1, d);
}

export function toDayKey(date: Date): string {
  const m = `${date.getMonth() + 1}`.padStart(2, '0');
  const d = `${date.getDate()}`.padStart(2, '0');
  return `${date.getFullYear()}-${m}-${d}`;
}

export function addDays(date: Date, days: number): Date {
  const out = new Date(date);
  out.setDate(out.getDate() + days);
  return out;
}

/** Local midnight today, so day arithmetic never straddles a timezone. */
export function today(): Date {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate());
}

/* ------------------------------------------------------------- heatmap */

export interface HeatCell {
  key: string;
  date: Date;
  label: string;
  words: number;
  dictations: number;
  /** Intensity bucket, or `none` for a day with no dictation. */
  level: 0 | 1 | 2 | 3 | 4;
}

export interface HeatGrid {
  /** One column per week, Sunday-first rows inside each. */
  weeks: HeatCell[][];
  /** Month label for each week column, empty when the month has not changed. */
  monthLabels: string[];
}

const WEEKDAYS = 7;

/**
 * Buckets a day list into week columns ending on the week that contains `end`.
 *
 * Dates come from the backend as `YYYY-MM-DD` and are parsed as local days, so
 * a week boundary lands where the user's own calendar puts it.
 */
export function buildHeatmap(days: DayCount[], weeks: number, end: Date): HeatGrid {
  const byDay = new Map(days.map((d) => [d.day, d]));

  // Walk back to the Sunday on or before `end`, then back `weeks` columns.
  const endSunday = addDays(end, -end.getDay());
  const firstSunday = addDays(endSunday, -(weeks - 1) * WEEKDAYS);

  // Intensity thresholds are relative to the busiest day in view, so a light
  // user still sees a gradient instead of one flat shade.
  let busiest = 0;
  for (const d of days) {
    if (d.words > busiest) busiest = d.words;
  }

  const grid: HeatCell[][] = [];
  const monthLabels: string[] = [];
  let prevMonth = '';

  for (let w = 0; w < weeks; w += 1) {
    const column: HeatCell[] = [];
    for (let dayOfWeek = 0; dayOfWeek < WEEKDAYS; dayOfWeek += 1) {
      const date = addDays(firstSunday, w * WEEKDAYS + dayOfWeek);
      const key = toDayKey(date);
      const hit = byDay.get(key);
      column.push({
        key,
        date,
        label: date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }),
        words: hit?.words ?? 0,
        dictations: hit?.dictations ?? 0,
        level: levelFor(hit?.words ?? 0, busiest),
      });
    }
    grid.push(column);

    // Label a column only where its month differs from the previous column's,
    // which is how a contribution graph reads.
    const month = addDays(firstSunday, w * WEEKDAYS).toLocaleDateString(undefined, {
      month: 'short',
    });
    monthLabels.push(month !== prevMonth ? month : '');
    prevMonth = month;
  }

  return { weeks: grid, monthLabels };
}

function levelFor(words: number, busiest: number): 0 | 1 | 2 | 3 | 4 {
  if (words <= 0 || busiest <= 0) return 0;
  const share = words / busiest;
  if (share >= 0.75) return 4;
  if (share >= 0.5) return 3;
  if (share >= 0.25) return 2;
  return 1;
}

/* ---------------------------------------------------- derived headline */

/** Words this month against last, as a signed percentage. */
export function monthChange(totals: Insights['totals']): number | null {
  const { words_this_month: now, words_last_month: before } = totals;
  // A month with no dictation has no baseline to compare against, so there is
  // no honest percentage to show.
  if (before <= 0) return null;
  return ((now - before) / before) * 100;
}

/** A plain-text summary of the real figures, for the copy button. */
export function summaryText(insights: Insights): string {
  const t = insights.totals;
  const lines = [
    `Orra — ${num(t.words)} words dictated across ${num(t.dictations)} dictations`,
  ];
  if (t.wpm !== null) lines.push(`${t.wpm.toFixed(0)} words per minute`);
  lines.push(`${insights.current_streak} day streak (longest ${insights.longest_streak})`);
  lines.push(`${num(t.vocabulary)} distinct words`);
  if (t.fixes > 0 || t.fillers > 0) {
    lines.push(`${num(t.fixes)} replacements and ${num(t.fillers)} fillers cleaned up`);
  }
  return lines.join('\n');
}
