import { useState, type ReactNode } from 'react';
import { AudioLines, BookOpenText, CalendarDays } from 'lucide-react';

import { AppUsageCard } from '@/components/AppUsageCard';
import { CircularShareBadge } from '@/components/CircularShareBadge';
import { DictatedCard } from '@/components/DictatedCard';
import { FixesCard } from '@/components/FixesCard';
import { SpeedometerGauge } from '@/components/SpeedometerGauge';
import { StreakCard } from '@/components/StreakCard';
import { Card, Empty } from '@/components/ui';
import { monthChange, num, summaryText } from '@/lib/stats';
import { cn } from '@/lib/utils';
import { useStore } from '@/store';

type Tab = 'usage' | 'voice';

/**
 * The figures the app can actually support.
 *
 * The mockup's third tab ranked you against invented people, and its numbers
 * were all made up; everything here is folded out of the real history in
 * `src-tauri/src/stats.rs`.
 */
export default function InsightsPage() {
  const { insights, config } = useStore();
  const [tab, setTab] = useState<Tab>('usage');

  if (!insights) {
    return (
      <Card>
        <Empty>Reading your history…</Empty>
      </Card>
    );
  }

  const t = insights.totals;
  const empty = t.dictations === 0;

  return (
    <>
      <div className="flex items-start justify-between mb-2">
        <h1 className="text-[26px] sm:text-[28px] font-bold tracking-tight">Insights</h1>
        <div className="shrink-0 -mt-1">
          <CircularShareBadge summary={summaryText(insights)} />
        </div>
      </div>

      <div role="tablist" className="flex items-center gap-7 border-b border-hair mb-6">
        <TabButton
          id="insights-tab-usage"
          panelId="insights-panel-usage"
          active={tab === 'usage'}
          onClick={() => setTab('usage')}
        >
          Your Usage
        </TabButton>
        <TabButton
          id="insights-tab-voice"
          panelId="insights-panel-voice"
          active={tab === 'voice'}
          onClick={() => setTab('voice')}
        >
          Your Voice
        </TabButton>
      </div>

      {empty && (
        <Card className="mb-5">
          <Empty>
            Nothing to measure yet — hold {config?.hotkey ?? 'your key'} and speak, and these
            fill in.
          </Empty>
        </Card>
      )}

      {tab === 'usage' && (
        <div
          role="tabpanel"
          id="insights-panel-usage"
          aria-labelledby="insights-tab-usage"
          className="grid grid-cols-1 lg:grid-cols-2 gap-5 sm:gap-6 items-start"
        >
          <div className="flex flex-col gap-5 sm:gap-6">
            <div className="grid grid-cols-1 sm:grid-cols-12 gap-5 sm:gap-6">
              <div className="sm:col-span-5 flex flex-col">
                <SpeedometerGauge wpm={t.wpm} measured={t.measured} />
              </div>
              <div className="sm:col-span-7 flex flex-col">
                <FixesCard replacements={t.fixes} fillers={t.fillers} />
              </div>
            </div>
            <AppUsageCard apps={insights.apps} />
          </div>

          <div className="flex flex-col gap-5 sm:gap-6">
            <DictatedCard
              words={t.words}
              change={monthChange(t)}
              books={t.books}
            />
            <StreakCard
              days={insights.days}
              currentStreak={insights.current_streak}
              longestStreak={insights.longest_streak}
            />
          </div>
        </div>
      )}

      {tab === 'voice' && (
        <div
          role="tabpanel"
          id="insights-panel-voice"
          aria-labelledby="insights-tab-voice"
          className="grid grid-cols-1 md:grid-cols-3 gap-5"
        >
          <VoiceStat
            icon={<AudioLines className="w-5 h-5" />}
            label="Recognition confidence"
            value={t.confidence === null ? '—' : `${(t.confidence * 100).toFixed(1)}%`}
            body={
              t.confidence === null
                ? 'No confidence score has been reported yet. Gemini does not report one; the other providers do.'
                : 'The mean confidence the transcription service reported across the words it heard.'
            }
          />
          <VoiceStat
            icon={<BookOpenText className="w-5 h-5" />}
            label="Vocabulary breadth"
            value={num(t.vocabulary)}
            body="Distinct words across every transcript, counted without case or punctuation."
          />
          <VoiceStat
            icon={<CalendarDays className="w-5 h-5" />}
            label="Days with dictation"
            value={num(insights.days.length)}
            body={
              insights.days.length === 0
                ? 'No day has a dictation on it yet.'
                : `${(t.dictations / insights.days.length).toFixed(1)} dictations on an average active day.`
            }
          />
        </div>
      )}
    </>
  );
}

/**
 * One tab of the Insights view.
 *
 * Handing the tab and its panel to each other by id is what lets a screen
 * reader say which of the two is showing. Arrow-key navigation between them is
 * deliberately absent: with two tabs, Tab already reaches both.
 */
function TabButton({
  id,
  panelId,
  active,
  onClick,
  children,
}: {
  id: string;
  panelId: string;
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      id={id}
      aria-controls={panelId}
      aria-selected={active}
      onClick={onClick}
      className={cn(
        'pb-2.5 text-[14px] sm:text-[15px] transition-all cursor-pointer relative',
        active
          ? 'font-bold text-ink'
          : 'font-semibold text-muted hover:text-ink',
      )}
    >
      {children}
      {active && <span className="absolute bottom-0 left-0 right-0 h-[2.5px] bg-ink rounded-full" />}
    </button>
  );
}

function VoiceStat({
  icon,
  label,
  value,
  body,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  body: string;
}) {
  return (
    <Card>
      <div className="flex items-center gap-2 text-teal dark:text-teal-soft mb-3">
        {icon}
        <span className="text-[12px] font-bold uppercase tracking-wider">{label}</span>
      </div>
      <div className="text-[36px] font-bold leading-none tracking-tight">{value}</div>
      <p className="text-[13px] text-muted mt-2">{body}</p>
    </Card>
  );
}
