import { Plus, Volume2 } from 'lucide-react';

import { ModelField } from '@/components/ModelField';
import {
  Button,
  Card,
  Divider,
  Empty,
  Label,
  PageHeading,
  Row,
  SectionTitle,
  Select,
  Toggle,
} from '@/components/ui';
import * as api from '@/lib/api';
import { LANGUAGES, STT_MODELS, TTS_VOICES, languageLabel } from '@/lib/deepgram-catalog';
import { languagesFor } from '@/lib/languages';
import { ASSEMBLYAI_LANGUAGES } from '@/lib/translate-catalog';
import { useStore } from '@/store';


/**
 * Which ears and which mouth the app uses.
 *
 * A `<select>` cannot show an option that is no longer in the catalogue, so a
 * value saved by another build is appended rather than silently dropped.
 */
function optionsWith(pairs: [string, string][], current: string, describe: boolean) {
  const opts = pairs.map(([value, label]) => (
    <option key={value} value={value}>
      {describe ? `${label} — ${value}` : label}
    </option>
  ));
  if (current && !pairs.some(([value]) => value === current)) {
    opts.push(
      <option key={current} value={current}>
        {describe ? `${current} — ${current}` : current}
      </option>,
    );
  }
  return opts;
}

export default function VoicePage() {
  const { config, status, mics, fail, update } = useStore();
  if (!config) return null;

  const cycle = config.language_cycle;
  // Model, language codes and keyterms are Deepgram's; the other providers
  // choose those for themselves, so showing the pickers would be a lie.
  const deepgram = config.provider === 'deepgram';
  const assemblyai = config.provider === 'assemblyai';
  // A server the user points at takes a model and a language; the two that
  // keep the model out of the request have neither to offer here.
  const provider = status?.providers.find((p) => p.value === config.provider);
  const selfHosted = provider?.self_hosted ?? false;
  const namesItsModel = provider?.has_model_list ?? false;
  const hasLanguage = provider?.has_language ?? false;

  // AssemblyAI steers towards one of eighteen languages and ignores the rest,
  // so its picker offers only those — and "automatic" for no steering at all,
  // which is what a code it cannot use amounts to anyway.
  const aaSupported = ASSEMBLYAI_LANGUAGES.some(([code]) => code === config.language);
  const aaLanguage = aaSupported ? config.language : '';

  const setCycle = (next: string[]) => update({ language_cycle: next });

  return (
    <>
      <PageHeading
        title="Voice"
        lede="Which ears and which mouth Orra uses."
      />

      <SectionTitle aside="Speech to text">Listening</SectionTitle>
      <Card className="flex flex-col gap-4">
        {deepgram || assemblyai ? (
          <div className={deepgram ? 'grid grid-cols-1 sm:grid-cols-2 gap-4' : undefined}>
            {deepgram && (
              <div>
                <Label htmlFor="stt-model">Model</Label>
                <Select
                  id="stt-model"
                  value={config.stt_model}
                  onChange={(e) => update({ stt_model: e.target.value })}
                >
                  {optionsWith(STT_MODELS, config.stt_model, false)}
                </Select>
              </div>
            )}
            <div>
              <Label htmlFor="language">Language</Label>
              {assemblyai ? (
                <Select
                  id="language"
                  value={aaLanguage}
                  onChange={(e) => update({ language: e.target.value })}
                >
                  <option value="">Automatic — no steering</option>
                  {ASSEMBLYAI_LANGUAGES.map(([code, label]) => (
                    <option key={code} value={code}>
                      {label} — {code}
                    </option>
                  ))}
                </Select>
              ) : (
                <Select
                  id="language"
                  value={config.language}
                  onChange={(e) => update({ language: e.target.value })}
                >
                  {optionsWith(LANGUAGES, config.language, true)}
                </Select>
              )}
              {assemblyai && (
                <p className="text-[12px] text-muted mt-1.5">
                  AssemblyAI steers its streaming model towards one of these eighteen, which is all
                  it listens in live.
                  {!aaSupported && config.language && (
                    <>
                      {' '}
                      <strong>{languageLabel(config.language)}</strong> is not one of them, so
                      dictation will transcribe without steering — Deepgram is the provider that
                      handles it.
                    </>
                  )}
                </p>
              )}
            </div>
          </div>
        ) : selfHosted ? (
          <div className="flex flex-col gap-4">
            {namesItsModel ? (
              <ModelField
                id="local-model"
                value={config.local_model}
                placeholder="whisper-large-v3"
                url={config.local_base_url}
                apiKey={config.local_key}
                help="Which model the server should transcribe with. Fetch models lists what it has; left empty, it uses whatever it was started with."
                onChange={(local_model) => update({ local_model })}
              />
            ) : (
              <p className="text-[12px] text-muted">
                {config.provider === 'orra'
                  ? 'The model is the one running on this machine — pick a different one under Settings → Transcription service.'
                  : 'This server transcribes with the model it was started with, so there is none to name here.'}
              </p>
            )}
            <div>
              <Label htmlFor="language">Language</Label>
              <Select
                id="language"
                value={config.language}
                onChange={(e) => update({ language: e.target.value })}
              >
                {optionsWith(languagesFor(config.provider), config.language, true)}
              </Select>
              <p className="text-[12px] text-muted mt-1.5">
                A hint, not a filter: a whisper model detects the language on its own, and picking
                one mostly saves it the guesswork. The spoken commands and replacements further down
                this page are English words either way.
              </p>
            </div>
          </div>
        ) : (
          <p className="text-[12px] text-muted">
            <strong>{api.providerLabel(config.provider)}</strong> chooses its own model and language,
            so the Deepgram model, language and keyterms settings are hidden. Switch provider under
            Settings → Transcription service.
          </p>
        )}

        <div>
          <Label htmlFor="mic">Microphone</Label>
          <Select id="mic" value={config.mic} onChange={(e) => update({ mic: e.target.value })}>
            <option value="">System default</option>
            {mics.map((mic) => (
              <option key={mic} value={mic}>
                {mic}
              </option>
            ))}
          </Select>
          {mics.length === 0 && (
            <p className="text-[12px] text-muted mt-1.5">
              No input devices could be listed, so the system default is used.
            </p>
          )}
        </div>

        {/* Shown wherever a language means something: these are what the switch
            key steps through, and the first one is where a fresh setup starts. */}
        {hasLanguage && (
          <>
            <Divider />

            <Row
              label="Languages to switch between"
              sub={
                <>
                  The language key steps through these. Add the codes you actually dictate in, such
                  as <code>en</code> for English or <code>fa</code> for Persian, and switch between
                  them with the key rather than through settings.
                </>
              }
            />

            <div className="flex flex-col">
              {cycle.length === 0 && <Empty>The language key has nothing to step through.</Empty>}
              {cycle.map((code, index) => (
                <div key={index} className="flex items-center gap-3 mt-3">
                  <Select
                    aria-label={`Language ${index + 1}`}
                    value={code}
                    onChange={(e) => {
                      const next = [...cycle];
                      next[index] = e.target.value;
                      setCycle(next);
                    }}
                  >
                    {optionsWith(LANGUAGES, code, true)}
                  </Select>
                  <Button
                    variant="mini"
                    className="shrink-0"
                    onClick={() => setCycle(cycle.filter((_, i) => i !== index))}
                  >
                    Remove
                  </Button>
                </div>
              ))}
            </div>

            <div>
              <Button onClick={() => setCycle([...cycle, LANGUAGES[0][0]])}>
                <Plus className="w-4 h-4" />
                Add language code
              </Button>
            </div>
          </>
        )}
      </Card>

      <SectionTitle aside="Text to speech">Speaking</SectionTitle>
      <Card className="flex flex-col gap-4">
        <Row
          label="Read aloud"
          sub="Enables the read-aloud buttons. Audio is generated by Deepgram Aura."
        >
          <Toggle
            label="Read aloud"
            checked={config.tts_enabled}
            onChange={(tts_enabled) => update({ tts_enabled })}
          />
        </Row>
        <Divider />
        <div>
          <Label htmlFor="tts-model">Voice</Label>
          <Select
            id="tts-model"
            value={config.tts_model}
            onChange={(e) => update({ tts_model: e.target.value })}
          >
            {optionsWith(TTS_VOICES, config.tts_model, false)}
          </Select>
        </div>
        <div>
          <Button
            disabled={!config.tts_enabled}
            onClick={async () => {
              try {
                await api.speak('This is Orra reading your words back to you.');
              } catch (e) {
                fail(api.problemOf(e));
              }
            }}
          >
            <Volume2 className="w-4 h-4" />
            Test voice
          </Button>
        </div>
      </Card>

      <p className="text-[12px] text-muted mt-4">
        {deepgram ? (
          <>
            Currently listening in <strong>{languageLabel(config.language)}</strong> with{' '}
            <strong>{config.stt_model}</strong>.
          </>
        ) : (
          <>
            Currently transcribing with{' '}
            <strong>
              {selfHosted && config.local_model
                ? `${config.local_model} on ${api.providerLabel(config.provider)}`
                : api.providerLabel(config.provider)}
            </strong>
            .
          </>
        )}
      </p>
    </>
  );
}
