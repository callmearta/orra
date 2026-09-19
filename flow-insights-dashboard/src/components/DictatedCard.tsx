import { TrendingDown, TrendingUp } from 'lucide-react';

import { Card, Divider } from '@/components/ui';
import { num } from '@/lib/stats';

/**
 * Total words dictated.
 *
 * The mockup's Desktop/Mobile split is gone: this is a Linux/macOS/Windows
 * desktop app and there is no mobile build to measure. The month-over-month
 * figure is real, and absent when there is no previous month to compare with.
 */
export function DictatedCard({
  words,
  change,
  books,
}: {
  words: number;
  change: number | null;
  books: number;
}) {
  const up = change !== null && change >= 0;

  return (
    <Card>
      <div className="flex items-start justify-between gap-3">
        <div className="text-[38px] sm:text-[42px] font-bold leading-none tracking-tight">
          {num(words)}
        </div>

        {change !== null && (
          <div
            className={`flex items-center gap-1 px-2.5 py-1 rounded-full text-[12px] font-bold tracking-tight ${
              up ? 'bg-good text-good-ink' : 'bg-amber-100 text-amber-900 dark:bg-amber-950 dark:text-amber-100'
            }`}
          >
            {up ? (
              <TrendingUp className="w-3.5 h-3.5" />
            ) : (
              <TrendingDown className="w-3.5 h-3.5" />
            )}
            <span>{Math.abs(Math.round(change))}% this month</span>
          </div>
        )}
      </div>

      <div className="text-[11px] font-semibold text-muted tracking-[0.08em] uppercase mt-2">
        Total words dictated
      </div>

      <Divider />

      <p className="text-[15px] sm:text-[16px] font-medium">
        {books < 1
          ? `${books === 0 ? 'No' : `${books.toFixed(2)} of a`} book written yet — 100,000 words makes one.`
          : `You have written ${books.toFixed(1)} complete ${books < 2 ? 'book' : 'books'}!`}
      </p>
    </Card>
  );
}
