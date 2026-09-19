import { Info } from 'lucide-react';

import { Card } from '@/components/ui';
import { cn } from '@/lib/utils';

/** The top of the gauge scale. Ordinary dictation runs well under this. */
const CEILING = 200;

/**
 * Words per minute, from the dictations that recorded a duration.
 *
 * There is no "top 0.1%" here any more: the mockup compared you against a
 * leaderboard that does not exist, and nothing in the app can know how fast
 * anyone else dictates.
 */
export function SpeedometerGauge({ wpm, measured }: { wpm: number | null; measured: number }) {
  const share = wpm === null ? 0 : Math.max(0, Math.min(1, wpm / CEILING));

  return (
    <Card className="flex flex-col justify-between h-full">
      <div>
        <div className="text-[38px] sm:text-[42px] font-bold leading-none tracking-tight">
          {wpm === null ? '—' : Math.round(wpm)}
        </div>
        <div className="text-[11px] font-semibold text-muted tracking-[0.08em] uppercase mt-2">
          Words per minute
        </div>
      </div>

      <div className="relative flex flex-col items-center justify-center pt-3 pb-1">
        <svg className="w-[180px] sm:w-[195px] h-[115px]" viewBox="0 0 200 135" fill="none">
          <path
            d="M 37 124 A 68 68 0 1 1 163 124"
            stroke="currentColor"
            className="text-hair"
            strokeWidth="14"
            strokeLinecap="round"
          />
          {/* pathLength normalises the arc to 100 units, so the dash array is
              the percentage of the scale directly. */}
          <path
            d="M 37 124 A 68 68 0 1 1 163 124"
            stroke="#264e4c"
            strokeWidth="14"
            strokeLinecap="round"
            pathLength={100}
            strokeDasharray={`${share * 100} 100`}
            className="transition-all duration-700"
          />
        </svg>

        <div className="absolute inset-0 flex flex-col items-center justify-center pt-6 pointer-events-none">
          <span className="text-[13px] font-medium text-muted leading-tight">of</span>
          <span className="text-[22px] sm:text-[24px] font-bold leading-none tracking-tight mt-0.5">
            {CEILING} wpm
          </span>
        </div>
      </div>

      <p className="flex items-start gap-1.5 text-[11px] text-muted mt-2">
        <Info className="w-3.5 h-3.5 shrink-0 mt-px" />
        <span>
          {measured === 0
            ? 'No dictation has been timed yet, so there is no rate to show.'
            : `Averaged over the ${measured} timed ${measured === 1 ? 'dictation' : 'dictations'}.`}
        </span>
      </p>
    </Card>
  );
}
