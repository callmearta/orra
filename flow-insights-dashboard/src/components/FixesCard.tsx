import { Info } from 'lucide-react';

import { Card, Divider } from '@/components/ui';
import { num } from '@/lib/stats';

/**
 * What the app changed on the way from speech to text.
 *
 * Both figures only count dictations made since these were recorded, so a
 * history that predates them reads as zero rather than as a guess.
 */
export function FixesCard({ replacements, fillers }: { replacements: number; fillers: number }) {
  const total = replacements + fillers;

  return (
    <Card className="flex flex-col justify-between h-full">
      <div>
        <div className="text-[38px] sm:text-[42px] font-bold leading-none tracking-tight">
          {num(total)}
        </div>
        <div className="text-[11px] font-semibold text-muted tracking-[0.08em] uppercase mt-2">
          Fixes made by Orra
        </div>
      </div>

      <Divider />

      <div className="flex flex-col gap-3.5 text-[14px] sm:text-[15px]">
        <div className="flex items-center justify-between gap-3">
          <div>
            <span className="font-bold">{num(fillers)}</span>{' '}
            <span className="font-medium">filler words dropped</span>
          </div>
          <span title="um, uh, hmm and friends, removed without breaking cadence">
            <Info className="w-4 h-4 text-muted shrink-0" />
          </span>
        </div>

        <div className="flex items-center justify-between gap-3">
          <div>
            <span className="font-bold">{num(replacements)}</span>{' '}
            <span className="font-medium">snippets applied</span>
          </div>
          <span title="How many times one of your replacement rules matched">
            <Info className="w-4 h-4 text-muted shrink-0" />
          </span>
        </div>
      </div>

      <p className="text-[11px] text-muted mt-4">
        Counted from the dictations made since Orra started recording this, so older ones
        contribute nothing.
      </p>
    </Card>
  );
}
