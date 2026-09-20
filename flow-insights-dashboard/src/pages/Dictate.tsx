import { CircleStop, Play, Square, Volume2 } from 'lucide-react';

import { LanguagePicker } from '@/components/LanguagePicker';
import { Button, Card, Meter, PageHeading, Pill, Row, SectionTitle } from '@/components/ui';
import * as api from '@/lib/api';
import { languageName, languagesFor } from '@/lib/languages';
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

  const setLanguage = async (code: string) => {
    try {
      await api.setLanguage(code);
      notify(`Language: ${languageName(code, languages)}`, 'ok');
    } catch (e) {
      fail(api.problemOf(e));
    }
  };

  const totals = insights?.totals;
  const provider = status?.providers.find((p) => p.value === config.provider);
  const selfHosted = provider?.self_hosted ?? false;
  // Empty for the providers that take no language — Gemini detects one, and
  // there is nothing to offer a picker for.
  const languages = languagesFor(config.provider);

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
              {selfHosted
                ? // The model is the part the user chose on a server they run,
                  // so that is the half worth naming.
                  `Transcribing with ${config.local_model || api.providerLabel(config.provider)}`
                : languages.length > 0
                  ? `Listening in ${languageName(config.language, languages)}`
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
            {languages.length > 0 && (
              <LanguagePicker
                current={config.language}
                options={languages}
                note={
                  selfHosted
                    ? 'A whisper model works out the language on its own; picking one mostly saves it the guesswork. The dictation language applies the moment you choose it.'
                    : 'The language the next dictation is transcribed in. Pick it here, or step through the list on the Voice page with the switch key.'
                }
                onPick={(code) => void setLanguage(code)}
              />
            )}
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
