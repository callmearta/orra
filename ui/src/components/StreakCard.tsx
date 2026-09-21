import { useState } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';

import { Card } from '@/components/ui';
import type { DayCount } from '@/lib/api';
import { addDays, buildHeatmap, num, today as localToday } from '@/lib/stats';
import { cn } from '@/lib/utils';

const WEEKS = 16;
const DAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** Intensity ramp, darkest first, matching the card's legend. */
const LEVEL_CLASSES: Record<number, string> = {
  0: 'bg-black/10 dark:bg-white/10',
  1: 'bg-[#c6e3df]',
  2: 'bg-[#74aba4]',
  3: 'bg-[#4e8781]',
  4: 'bg-[#264e4c]',
};

/**
 * The dictation streak and its calendar.
 *
 * Every cell is a real local day, and the arrows page the window back through
 * history — in the mockup they were decorative.
 */
export function StreakCard({
  days,
  currentStreak,
  longestStreak,
}: {
  days: DayCount[];
  currentStreak: number;
  longestStreak: number;
}) {
  const [weeksBack, setWeeksBack] = useState(0);
  const [hovered, setHovered] = useState<string | null>(null);

  const end = addDays(localToday(), -weeksBack * 7);
  const grid = buildHeatmap(days, WEEKS, end);

  // Days making up the run that is still alive, so they can be outlined. Read
  // once rather than per cell, since every cell asks about the same instant.
  const now = localToday();
  const streakStart = addDays(now, -(currentStreak - 1));
  const inCurrentStreak = (date: Date) =>
    currentStreak > 0 &&
    date.getTime() >= streakStart.getTime() &&
    date.getTime() <= now.getTime();

  const hoveredCell = hovered
    ? grid.weeks.flat().find((cell) => cell.key === hovered) ?? null
    : null;

  return (
    <Card>
      <div className="flex items-center justify-between gap-3 mb-5">
        <h3 className="text-[22px] sm:text-[24px] font-bold tracking-tight">
          {currentStreak === 0
            ? 'No streak yet'
            : `${currentStreak} day streak`}
        </h3>
        <span className="text-[11px] font-semibold text-muted tracking-[0.08em] uppercase">
          Longest streak | {longestStreak} {longestStreak === 1 ? 'day' : 'days'}
        </span>
      </div>

      <div className="flex items-center text-muted mb-2.5">
        <button
          type="button"
          className="p-0.5 hover:text-ink transition-colors cursor-pointer shrink-0"
          title="Earlier weeks"
          aria-label="Earlier weeks"
          onClick={() => setWeeksBack((w) => w + WEEKS)}
        >
          <ChevronLeft className="w-4 h-4" />
        </button>
        <div className="flex-1 mx-3 grid grid-cols-[repeat(16,minmax(0,1fr))] text-center text-[12px] font-semibold select-none">
          {grid.monthLabels.map((label, i) => (
            <span key={i}>{label}</span>
          ))}
        </div>
        <button
          type="button"
          className="p-0.5 hover:text-ink transition-colors cursor-pointer shrink-0 disabled:opacity-30 disabled:pointer-events-none"
          title="Later weeks"
          aria-label="Later weeks"
          disabled={weeksBack === 0}
          onClick={() => setWeeksBack((w) => Math.max(0, w - WEEKS))}
        >
          <ChevronRight className="w-4 h-4" />
        </button>
      </div>

      <div className="flex items-start gap-3 overflow-x-auto pb-1">
        <div className="flex flex-col justify-between h-[154px] text-[11px] font-semibold text-muted pt-[2px] select-none shrink-0">
          {DAY_LABELS.map((day) => (
            <span key={day} className="leading-none h-[18px] flex items-center">
              {day}
            </span>
          ))}
        </div>

        <div className="flex-1 grid grid-cols-[repeat(16,minmax(0,1fr))] gap-[5px] min-w-[420px]">
          {grid.weeks.map((week, weekIndex) => (
            <div key={weekIndex} className="flex flex-col gap-[5px]">
              {week.map((cell) => {
                const future = cell.date.getTime() > now.getTime();
                return (
                  <div
                    key={cell.key}
                    onMouseEnter={() => setHovered(cell.key)}
                    onMouseLeave={() => setHovered(null)}
                    title={`${cell.label}: ${cell.words > 0 ? `${num(cell.words)} words` : 'nothing dictated'}`}
                    className={cn(
                      'w-full aspect-square rounded-[5px] transition-all duration-150',
                      future ? 'bg-transparent' : LEVEL_CLASSES[cell.level],
                      inCurrentStreak(cell.date) &&
                        cell.words > 0 &&
                        'ring-2 ring-ink dark:ring-[#74aba4]',
                    )}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>

      <div className="flex items-center justify-between text-[11px] font-semibold text-muted mt-4">
        <div className="flex items-center gap-1.5">
          <span>More</span>
          {[4, 3, 2, 1].map((level) => (
            <span key={level} className={cn('w-3.5 h-3.5 rounded-[3px]', LEVEL_CLASSES[level])} />
          ))}
          <span>Less</span>
        </div>

        <div className="flex items-center gap-2 text-ink">
          <span className="w-3.5 h-3.5 rounded-[3px] ring-2 ring-ink dark:ring-[#74aba4]" />
          <span>Current streak</span>
        </div>
      </div>

      <div className="text-[12px] text-muted mt-3 min-h-[18px]">
        {hoveredCell
          ? `${hoveredCell.label} — ${
              hoveredCell.words > 0
                ? `${num(hoveredCell.words)} words in ${hoveredCell.dictations} ${
                    hoveredCell.dictations === 1 ? 'dictation' : 'dictations'
                  }`
                : 'nothing dictated'
            }`
          : weeksBack > 0
            ? `Showing the ${WEEKS} weeks ending ${end.toLocaleDateString(undefined, { month: 'long', day: 'numeric', year: 'numeric' })}`
            : 'Each square is one day.'}
      </div>
    </Card>
  );
}
