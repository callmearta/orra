import { CircleStop, Play, Square, Volume2 } from 'lucide-react';

import { Button, Card, Meter, PageHeading, Pill, Row, SectionTitle } from '@/components/ui';
import * as api from '@/lib/api';
import { languageLabel } from '@/lib/deepgram-catalog';
import { num } from '@/lib/stats';
import { useStore } from '@/store';

export default function DictatePage({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { config, status, insights, history, live, notify, fail } = useStore();
  if (!config) return null;

  const recording = live.phase === 'recording';
  const busy = recording || live.phase === 'processing';
  const speaking = status?.speaking ?? false;

  const phase = speaking ? 'Speaking' : recording ? 'Listening' : busy ? 'Finishing' : 'Ready';

  const dictate = async () => {
    try {
      await (recording ? api.stopDictation() : api.startDictation());
    } catch (e) {
      fail(api.problemOf(e));
    }
  };

  const readAloud = async () => {
    const latest = history[0];
    if (!latest) return;
    try {
      await api.speak(latest.text);
    } catch (e) {
      fail(api.problemOf(e));
    }
  };

  const switchLanguage = async () => {
    try {
      notify(`Language: ${languageLabel(await api.cycleLanguage())}`, 'ok');
    } catch (e) {
      fail(api.problemOf(e));
    }
  };

  const totals = insights?.totals;
  // The language belongs to Deepgram: it is the only provider here that cannot
  // detect one, and the only one that takes a code at all.
  const deepgram = config.provider === 'deepgram';

  return (
    <>
      <PageHeading
        title="Dictate"
        lede="Hold your key, speak, and let go. The text lands in whatever window is focused."
      />

      {status && !status.has_key && (
        <Card className="mt-6 border border-amber-300 dark:border-amber-900">
          <Row
            label={`No ${api.providerLabel(config.provider)} API key`}
            sub="Dictation cannot start until a key is available. Add one in Settings."
          >
            <Button variant="primary" onClick={onOpenSettings}>
              Open settings
            </Button>
          </Row>
        </Card>
      )}

      <Card className="mt-6">
        <div className="flex flex-col items-center gap-4 py-2">
          <div className="flex items-center gap-3">
            <Pill tone={busy || speaking ? 'good' : 'neutral'}>
              <span
                className={`w-[7px] h-[7px] rounded-full ${
                  recording ? 'bg-teal animate-pulse' : busy || speaking ? 'bg-teal-mid' : 'bg-emerald-400'
                }`}
              />
              {phase}
            </Pill>
            <span className="text-[12px] text-muted">
              {deepgram
                ? `Listening in ${languageLabel(config.language)}`
                : `Transcribing with ${api.providerLabel(config.provider)}`}
            </span>
          </div>

          <Meter level={live.level} active={recording} />

          <div className="w-full min-h-[86px] max-h-[160px] overflow-y-auto bg-canvas rounded-2xl px-5 py-4 text-[15px] leading-relaxed">
            {live.final || live.interim ? (
              <>
                {live.final}
                {live.interim && (
                  <span className="text-muted">
                    {live.final ? ' ' : ''}
                    {live.interim}
                  </span>
                )}
              </>
            ) : (
              <span className="text-muted italic">
                Nothing yet — your words will appear here as you speak.
              </span>
            )}
          </div>

          <div className="flex flex-wrap items-center justify-center gap-2.5">
            <Button variant="primary" onClick={dictate}>
              {recording ? <Square className="w-4 h-4" /> : <Play className="w-4 h-4" />}
              {recording ? 'Stop dictation' : 'Start dictation'}
            </Button>
            <Button onClick={readAloud} disabled={history.length === 0}>
              <Volume2 className="w-4 h-4" />
              Read last aloud
            </Button>
            {speaking && (
              <Button onClick={() => void api.stopSpeaking()}>
                <CircleStop className="w-4 h-4" />
                Stop reading
              </Button>
            )}
            {deepgram && <Button onClick={switchLanguage}>Switch language</Button>}
          </div>
        </div>
      </Card>

      <SectionTitle aside={config.hotkey}>Counts so far</SectionTitle>
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <Stat label="Words dictated" value={totals ? num(totals.words) : '—'} />
        <Stat label="Dictations" value={totals ? num(totals.dictations) : '—'} />
        <Stat
          label="Typing time saved"
          value={totals ? `${num(totals.minutes_saved)}m` : '—'}
          sub="at 40 words per minute"
        />
      </div>
    </>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <Card className="text-center">
      <div className="text-[28px] font-bold leading-none tracking-tight">{value}</div>
      <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted mt-2">
        {label}
      </div>
      {sub && <div className="text-[11px] text-muted mt-1">{sub}</div>}
    </Card>
  );
}
