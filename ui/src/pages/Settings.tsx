import { useCallback, useEffect, useState } from 'react';
import { Download, LogOut, Play, RotateCw, ShieldCheck, Square } from 'lucide-react';

import { HotkeyInput } from '@/components/HotkeyInput';
import { ModelField } from '@/components/ModelField';
import {
  Button,
  Card,
  Divider,
  FieldError,
  Input,
  Label,
  LocalProgressStrip,
  PageHeading,
  Pill,
  Row,
  SectionTitle,
  Select,
  Toggle,
} from '@/components/ui';
import * as api from '@/lib/api';
import { TRANSLATE_LANGUAGES, TRANSLATE_MODELS } from '@/lib/translate-catalog';
import { blank, httpUrl, required } from '@/lib/validation';
import { useStore } from '@/store';

/**
 * What each provider is, in one line — including which model it will use, since
 * two of them pick that themselves and nothing else in the interface says so.
 */
const PROVIDER_BLURB: Record<api.Provider, string> = {
  deepgram: 'nova-3 and the other Deepgram models. Its model, language and keyterms are on the Voice page.',
  assemblyai: 'AssemblyAI hears the audio itself and picks its own model, so the Deepgram settings on the Voice page do not apply.',
  gemini: 'Streams with gemini-3.5-transcribe-live. It detects the language itself and takes no keyterms, so the Deepgram settings on the Voice page do not apply.',
  ollama:
    'Ollama on this machine, for its audio-capable models. Its chat models cannot transcribe — a whisper server is still the safe choice.',
  speaches:
    'Speaches, formerly faster-whisper-server. Takes HTTP requests, streams transcriptions back, and can hold a live socket.',
  localai: 'LocalAI. The whole OpenAI surface, including the streaming transcriptions and the Realtime socket.',
  whispercpp:
    'A whisper.cpp server. It transcribes with the model it was started with, so there is no model to name here — only the address — and its own path is used exactly as typed.',
  local:
    'Any other server speaking one of these APIs. Give it a URL, and say which of the three shapes it answers on.',
  orra: 'A model Orra downloads and runs on this machine itself. Nothing to install first, and the settings below are filled in for you.',
};

/**
 * Fetching and running a model on this machine.
 *
 * The alternative to everything above it in the card: instead of pointing at a
 * server the user set up, this fetches whisper.cpp and a model, starts the
 * server itself and fills in the settings to match. Hidden where no build is
 * published for the platform, rather than offering a button that cannot work.
 */
function LocalEngine({ onSettled }: { onSettled: () => void }) {
  const { fail, localBusy, localProgress, downloadLocalModel, useLocalModel, stopLocalEngine } = useStore();
  const [info, setInfo] = useState<api.LocalAvailability | null>(null);
  const [choice, setChoice] = useState<string>('');

  const refresh = useCallback(async () => {
    try {
      const next = await api.localAvailability();
      setInfo(next);
      // Whatever is running, or failing that the smallest thing already on
      // disk: a second visit should not have to pick again.
      setChoice((current) => {
        if (current) return current;
        return next.running_model ?? next.models.find((m) => m.installed)?.name ?? 'small';
      });
    } catch (e) {
      fail(api.problemOf(e));
    }
  }, [fail]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    // The engine can come up without anything being pressed here: at launch it
    // starts itself, and switching the provider to this one starts it too. Both
    // are announced on `status`, so the card can stop saying "nothing is
    // running" the moment that stops being true.
    let stop: (() => void) | undefined;
    void api.listen('status', () => void refresh()).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }, [refresh]);

  if (!info?.supported) return null;

  const chosen = info.models.find((m) => m.name === choice);
  const chosenRunning = info.running && info.running_model === choice;
  const running = info.running_model
    ? info.models.find((m) => m.name === info.running_model)?.label ?? info.running_model
    : null;

  const act = async (what: 'download' | 'use' | 'stop') => {
    if (what === 'stop') await stopLocalEngine();
    else if (what === 'download') await downloadLocalModel(choice);
    else await useLocalModel(choice);
    onSettled();
    void refresh();
  };

  return (
    <>
      <Divider />
      <div className="flex flex-col gap-3">
        <div>
          <Label htmlFor="local-engine">Run a model on this machine</Label>
          <div className="flex items-end gap-3">
            <Select
              id="local-engine"
              className="flex-1"
              value={choice}
              onChange={(e) => setChoice(e.target.value)}
            >
              {info.models.map((m) => (
                <option key={m.name} value={m.name}>
                  {m.label} — {m.mb} MB{m.installed ? ' (downloaded)' : ''}
                </option>
              ))}
            </Select>
            {/* Download first, start second: the weights are worth keeping
                whether or not the model is running, and Stop must not take
                them with it. */}
            {chosenRunning ? (
              <Button
                variant="primary"
                status="active"
                statusText="Running"
                disabled
                className="disabled:opacity-100"
              >
                <Play className="w-4 h-4" />
                Running
              </Button>
            ) : chosen?.installed ? (
              <Button
                variant="primary"
                onClick={() => void act('use')}
                loading={localBusy === 'use'}
                loadingText="Starting…"
                disabled={localBusy !== null}
              >
                <Play className="w-4 h-4" />
                Use this model
              </Button>
            ) : (
              <Button
                variant="primary"
                onClick={() => void act('download')}
                loading={localBusy === 'download'}
                loadingText="Downloading…"
                disabled={localBusy !== null}
              >
                <Download className="w-4 h-4" />
                Download
              </Button>
            )}
            {info.running && (
              <Button
                onClick={() => void act('stop')}
                loading={localBusy === 'stop'}
                loadingText="Stopping…"
                disabled={localBusy !== null}
              >
                <Square className="w-4 h-4" />
                Stop
              </Button>
            )}
          </div>
          <p className="text-[12px] text-muted mt-1.5">
            {chosen?.note} Orra fetches it, starts the server and fills in the settings above —
            nothing to install first, and nothing added to the app bundle.
          </p>
        </div>
        {localProgress ? (
          <LocalProgressStrip progress={localProgress} />
        ) : (
          <p className="text-[12px] text-muted">
            {localBusy === 'download'
              ? 'Preparing the download…'
              : localBusy === 'use'
                ? 'Starting the engine — loading the model takes a moment…'
                : running
                  ? `Running ${running}.`
                  : 'Nothing is running from here.'}
          </p>
        )}
      </div>
    </>
  );
}

export default function SettingsPage() {
  const { config, status, notify, fail, update, updateNow, refreshStatus } = useStore();
  const [keyStatus, setKeyStatus] = useState<string | null>(null);
  const [translateStatus, setTranslateStatus] = useState<string | null>(null);
  const [keyChecking, setKeyChecking] = useState(false);
  const [translateChecking, setTranslateChecking] = useState(false);
  const [separateTranslateKey, setSeparateTranslateKey] = useState(
    () => !blank(config?.translate_gemini_key ?? ''),
  );
  const [applyingHotkey, setApplyingHotkey] = useState(false);
  const [hotkeyApplied, setHotkeyApplied] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [quitBusy, setQuitBusy] = useState(false);

  if (!config) return null;

  const mode = config.mode;
  // What the backend says this provider is: whether it is one the user hosts,
  // which field holds its key, and what it can be asked for. The config only
  // exists once the status has arrived, so this is always the right one.
  const providers = status?.providers ?? [];
  const provider = providers.find((p) => p.value === config.provider);
  const keyField: api.KeyField = provider?.key_field ?? 'api_key';
  const hotkeyError = required(config.hotkey, 'Push-to-talk key');
  const localUrlError =
    provider?.self_hosted && config.provider !== 'orra' ? httpUrl(config.local_base_url, 'Endpoint') : null;
  const providerKeyError =
    provider && !provider.self_hosted && !status?.has_key && blank(config[keyField])
      ? `${api.providerLabel(config.provider)} API key is required.`
      : null;
  const presetEndpoints = providers.filter(
    (p) => p.preset_url && p.value !== config.provider && p.value !== 'orra',
  );
  const translateUrlError =
    config.translate_provider === 'custom' ? httpUrl(config.translate_base_url, 'API URL') : null;
  const translateModelError =
    config.translate_provider === 'custom'
      ? required(config.translate_custom_model, 'Model')
      : null;
  const useTranscriptionKey = !separateTranslateKey;
  const translateGeminiKeyError =
    config.translate_provider === 'gemini' && separateTranslateKey && blank(config.translate_gemini_key)
      ? 'Enter a Gemini API key, or turn on Use transcription key.'
      : null;

  return (
    <>
      <PageHeading
        title="Settings"
        lede="Trigger, delivery and behaviour. Everything here saves as you change it."
        aside={status?.hyprland ? <Pill tone="neutral">Hyprland</Pill> : undefined}
      />

      <SectionTitle>Push to talk</SectionTitle>
      <Card className="flex flex-col gap-4">
        <div>
          <Label htmlFor="hotkey">Key</Label>
          <HotkeyInput
            label="Push to talk key"
            value={config.hotkey}
            invalid={!!hotkeyError}
            onChange={(hotkey) => {
              setHotkeyApplied(false);
              void updateNow({ hotkey });
            }}
          />
          <FieldError>{hotkeyError}</FieldError>
          {!hotkeyError && (
            <p className="text-[12px] text-muted mt-1.5">
              Click the box, then press the combination you want.
            </p>
          )}
        </div>

        <Row
          label="Mode"
          sub={
            mode === 'toggle'
              ? 'Tap once to start, tap again to stop.'
              : 'Hold the key while you speak, release to insert.'
          }
        >
          <Select
            className="w-auto"
            aria-label="Trigger mode"
            value={mode}
            onChange={(e) => void updateNow({ mode: e.target.value as 'hold' | 'toggle' })}
          >
            <option value="hold">Hold to talk</option>
            <option value="toggle">Tap to toggle</option>
          </Select>
        </Row>

        {/* Steps through the language list on the Voice page. Shown wherever a
            language means something — Gemini detects one for itself. */}
        {provider?.has_language && (
          <div>
            <Label htmlFor="language-hotkey">Switch language key</Label>
            <HotkeyInput
              label="Switch language key"
              value={config.language_hotkey}
              onChange={(language_hotkey) => {
                setHotkeyApplied(false);
                void updateNow({ language_hotkey });
              }}
            />
            <p className="text-[12px] text-muted mt-1.5">
              Steps through the languages listed under Voice. A whisper server detects the language
              on its own, so this is a shortcut rather than a requirement.
            </p>
          </div>
        )}

        <Divider />

        <Row
          label={status?.hyprland ? 'Hyprland binding' : 'System shortcut'}
          sub={
            status?.hyprland
              ? 'Written into keybinds.lua and reloaded — Hyprland owns this shortcut.'
              : 'Registered with the system through the app.'
          }
        >
          <Button
            loading={applyingHotkey}
            loadingText="Applying…"
            status={hotkeyApplied ? 'success' : 'idle'}
            statusText="Applied"
            disabled={!!hotkeyError}
            onClick={async () => {
              setApplyingHotkey(true);
              setHotkeyApplied(false);
              try {
                notify(await api.applyHotkey(), 'ok');
                setHotkeyApplied(true);
              } catch (e) {
                fail(api.problemOf(e));
              } finally {
                setApplyingHotkey(false);
              }
            }}
          >
            <RotateCw className="w-4 h-4" />
            Re-apply
          </Button>
        </Row>
      </Card>

      <SectionTitle>Delivery</SectionTitle>
      <Card className="flex flex-col gap-4">
        <div>
          <Label htmlFor="injection">How the text gets inserted</Label>
          <Select
            id="injection"
            value={config.injection}
            onChange={(e) =>
              void updateNow({ injection: e.target.value as 'clipboard-paste' | 'type' })
            }
          >
            <option value="clipboard-paste">Clipboard then paste (most reliable)</option>
            <option value="type">Type it out (leaves the clipboard alone)</option>
          </Select>
        </div>
        <Divider />
        <Row label="Restore clipboard afterwards" sub="Put back whatever you had copied before dictating.">
          <Toggle
            label="Restore clipboard afterwards"
            checked={config.restore_clipboard}
            onChange={(restore_clipboard) => update({ restore_clipboard })}
          />
        </Row>
        <Divider />
        <Row label="Trailing space" sub="So back-to-back dictations do not run into each other.">
          <Toggle
            label="Trailing space"
            checked={config.trailing_space}
            onChange={(trailing_space) => update({ trailing_space })}
          />
        </Row>
        <Divider />
        <Row label="Press Enter when finished" sub="Send the message as soon as the text lands.">
          <Toggle
            label="Press Enter when finished"
            checked={config.auto_submit}
            onChange={(auto_submit) => update({ auto_submit })}
          />
        </Row>
      </Card>

      <SectionTitle>Interface</SectionTitle>
      <Card className="flex flex-col gap-4">
        <Row label="Floating transcript" sub="A small overlay that follows you while you speak.">
          <Toggle label="Floating transcript" checked={config.hud} onChange={(hud) => update({ hud })} />
        </Row>
        <Divider />
        <Row label="Sound cues" sub="A soft tone when recording starts and stops.">
          <Toggle
            label="Sound cues"
            checked={config.sounds}
            onChange={(sounds) => update({ sounds })}
          />
        </Row>
        <Divider />
        <Row label="Start at login" sub="Launch Orra automatically so the key always works.">
          <Toggle
            label="Start at login"
            checked={config.launch_at_login}
            onChange={(launch_at_login) => update({ launch_at_login })}
          />
        </Row>
        <Divider />
        <Row label="Dark mode" sub="Remembered in the config file, so it survives a restart.">
          <Toggle
            label="Dark mode"
            checked={config.dark_mode}
            onChange={(dark_mode) => update({ dark_mode })}
          />
        </Row>
      </Card>

      <SectionTitle>Transcription service</SectionTitle>
      <Card className="flex flex-col gap-3">
        <div>
          <Label htmlFor="provider">Provider</Label>
          <Select
            id="provider"
            value={config.provider}
            onChange={(e) => {
              const next = e.target.value as api.Provider;
              const preset = providers.find((p) => p.value === next)?.preset_url;
              // The last check was about the provider being switched away from.
              setKeyStatus(null);
              // A named server is named for where it answers, so choosing one
              // fills the address in — that is the whole point of listing it.
              // Anything typed over it is the user's to keep, until they pick
              // another one. HTTP with it: it is what every one of these
              // answers, and a transport left over from another provider is a
              // dictation that fails for no visible reason.
              void updateNow(
                preset
                  ? { provider: next, local_base_url: preset, local_transport: 'http' }
                  : { provider: next },
              );
            }}
          >
            {api.PROVIDERS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </Select>
          <p className="text-[12px] text-muted mt-1.5">{PROVIDER_BLURB[config.provider]}</p>
        </div>

        {provider?.self_hosted && (
          <>
            {config.provider === 'orra' ? (
              // Everything above is filled in by this: the address, the
              // transport, and which model is loaded.
              <LocalEngine onSettled={() => void refreshStatus()} />
            ) : (
              <>
                <div>
                  <Label htmlFor="local-url">Endpoint</Label>
                  <Input
                    id="local-url"
                    placeholder="http://localhost:8000/v1"
                    value={config.local_base_url}
                    invalid={!!localUrlError}
                    aria-describedby="local-url-help"
                    onChange={(e) => {
                      setKeyStatus(null);
                      update({ local_base_url: e.target.value });
                    }}
                  />
                  {presetEndpoints.length > 0 && (
                    <div className="flex flex-wrap gap-1.5 mt-2">
                      {presetEndpoints.map((p) => (
                        <button
                          key={p.value}
                          type="button"
                          onClick={() => {
                            setKeyStatus(null);
                            update({ local_base_url: p.preset_url!, local_transport: 'http' });
                          }}
                          className="rounded-full border border-hair px-2.5 py-1 text-[11px] font-semibold text-muted hover:text-ink hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer"
                        >
                          {p.label}
                        </button>
                      ))}
                    </div>
                  )}
                  <FieldError>{localUrlError}</FieldError>
                  {!localUrlError && (
                    <p id="local-url-help" className="text-[12px] text-muted mt-1.5">
                      Where the server answers. <code>/audio/transcriptions</code> is added for you,
                      unless the URL already ends at <code>/inference</code>.
                    </p>
                  )}
                </div>

                {provider.has_transport_choice && (
                  <Row
                    label="Type"
                    sub={
                      api.LOCAL_TRANSPORTS.find(([value]) => value === config.local_transport)?.[2] ??
                      ''
                    }
                  >
                    <Select
                      className="w-auto"
                      aria-label="Transport"
                      value={config.local_transport}
                      onChange={(e) =>
                        void updateNow({ local_transport: e.target.value as api.LocalTransport })
                      }
                    >
                      {api.LOCAL_TRANSPORTS.map(([value, label]) => (
                        <option key={value} value={value}>
                          {label}
                        </option>
                      ))}
                    </Select>
                  </Row>
                )}

                {provider.has_model_list && (
                  <ModelField
                    id="local-model"
                    value={config.local_model}
                    placeholder="whisper-large-v3"
                    url={config.local_base_url}
                    apiKey={config.local_key}
                    help="Which model the server should transcribe with. Fetch models lists what it has."
                    onChange={(local_model) => update({ local_model })}
                  />
                )}
              </>
            )}
          </>
        )}

        {/* No key for the model Orra runs itself: there is nothing between two
            processes on loopback to authenticate. */}
        {config.provider !== 'orra' && (
          <div>
            <Label htmlFor="provider-key">
              {provider?.self_hosted
                ? 'API key (optional)'
                : `${api.providerLabel(config.provider)} API key`}
            </Label>
            <Input
              id="provider-key"
              type="password"
              placeholder={
                provider?.self_hosted
                  ? 'Only if your server asks for one'
                  : 'Read from .env — paste here to override'
              }
              value={config[keyField]}
              invalid={!!providerKeyError}
              onChange={(e) => update({ [keyField]: e.target.value })}
            />
            <FieldError>{providerKeyError}</FieldError>
            {!providerKeyError && (
              <p className="text-[12px] text-muted mt-1.5">
                {provider?.self_hosted ? (
                <>
                  Sent as a <code>Bearer</code> token. Left empty, no authorization header goes out
                  at all, which is what a server on this machine normally wants.
                </>
              ) : (
                <>
                  Resolution order is environment, then the nearest <code>.env</code>, then this
                  value. Read aloud always uses Deepgram's voices, so it needs a Deepgram key too.
                </>
                )}
              </p>
            )}
          </div>
        )}

        <div className="flex items-center gap-3">
          <Button
            loading={keyChecking}
            loadingText="Checking…"
            status={keyStatus ? 'success' : 'idle'}
            statusText={provider?.self_hosted ? 'Server OK' : 'Verified'}
            disabled={!!localUrlError || !!providerKeyError}
            onClick={async () => {
              setKeyChecking(true);
              try {
                // The fields above save on a debounce, so a check clicked
                // straight after typing would otherwise test the values from
                // before — which for a self-hosted provider is the whole
                // address, and for the engine is whether it came up at all.
                await updateNow({});
                const message = await api.verifyKey();
                setKeyStatus(message);
                notify(
                  provider?.self_hosted
                    ? 'The server answered'
                    : `${api.providerLabel(config.provider)} key verified`,
                  'ok',
                );
              } catch (e) {
                // Cleared rather than filled in: the card above carries the
                // whole failure, and this line would only repeat its summary —
                // which for this command is the static "Checking the … key",
                // reading as though the check were still running.
                setKeyStatus(null);
                fail(api.problemOf(e));
              } finally {
                setKeyChecking(false);
              }
            }}
          >
            <ShieldCheck className="w-4 h-4" />
            {provider?.self_hosted ? 'Check server' : 'Verify key'}
          </Button>
          <span className="text-[12px] text-muted">
            {keyStatus ??
              (provider?.self_hosted
                ? 'Transcribes a moment of silence, so a wrong address or model is found here.'
                : status?.has_key
                  ? 'A key is available.'
                  : 'No key found.')}
          </span>
        </div>
      </Card>

      <SectionTitle>Translation</SectionTitle>
      <Card className="flex flex-col gap-4">
        <div>
          <Label htmlFor="translate-hotkey">Translate key</Label>
          <HotkeyInput
            label="Translate key"
            value={config.translate_hotkey}
            onChange={(translate_hotkey) => void updateNow({ translate_hotkey })}
          />
          <p className="text-[12px] text-muted mt-1.5">
            {mode === 'toggle'
              ? 'Tap it to dictate a translation, tap again to finish.'
              : 'Hold it and speak, and the translation is typed instead of the words.'}{' '}
            Left empty, there is no translating key at all.
          </p>
        </div>

        <Divider />

        <Row
          label="Translate into"
          sub="The transcript is translated into this language before it is typed."
        >
          <Select
            className="w-auto"
            aria-label="Translate into"
            value={config.translate_language}
            onChange={(e) => update({ translate_language: e.target.value })}
          >
            {TRANSLATE_LANGUAGES.map(([code, label]) => (
              <option key={code} value={code}>
                {label}
              </option>
            ))}
          </Select>
        </Row>

        <Divider />

        <div>
          <Label htmlFor="translate-provider">Service</Label>
          <Select
            id="translate-provider"
            value={config.translate_provider}
            onChange={(e) => {
              // The last check was about the service being switched away from.
              setTranslateStatus(null);
              void updateNow({ translate_provider: e.target.value as api.TranslateProvider });
            }}
          >
            <option value="gemini">Gemini</option>
            <option value="custom">Custom endpoint (OpenAI-compatible)</option>
          </Select>
          <p className="text-[12px] text-muted mt-1.5">
            {config.translate_provider === 'gemini'
              ? 'Whichever provider is transcribing, translating runs on Gemini — with its own key just below.'
              : 'Any service that speaks the OpenAI /chat/completions API: OpenAI, OpenRouter, Groq, or a model running on this machine.'}
          </p>
        </div>

        {config.translate_provider === 'gemini' ? (
          <>
            <Row label="Model" sub="A flash model is plenty — one short instruction and a paragraph of text.">
              <Select
                className="w-auto"
                aria-label="Translation model"
                value={config.translate_model}
                onChange={(e) => {
                  // A different model is a different thing to check.
                  setTranslateStatus(null);
                  update({ translate_model: e.target.value });
                }}
              >
                {TRANSLATE_MODELS.map(([id, label]) => (
                  <option key={id} value={id}>
                    {label}
                  </option>
                ))}
              </Select>
            </Row>

            <Row
              label="Use transcription key"
              sub="Translate with the Gemini key already configured for transcription, or fall back to GEMINI_API_KEY."
            >
              <Toggle
                label="Use transcription key"
                checked={useTranscriptionKey}
                onChange={(next) => {
                  setTranslateStatus(null);
                  setSeparateTranslateKey(!next);
                  if (next) update({ translate_gemini_key: '' });
                }}
              />
            </Row>

            {/* A field of its own, because Gemini translating while something
                else transcribes is the ordinary case: the Gemini key on the
                transcription card is not even shown then, so a key wanted only
                for translating would have nowhere to go. */}
            {!useTranscriptionKey && (
              <div>
                <Label htmlFor="translate-gemini-key">Gemini API key</Label>
                <Input
                  id="translate-gemini-key"
                  type="password"
                  placeholder="Paste a Gemini key for translation"
                  value={config.translate_gemini_key}
                  invalid={!!translateGeminiKeyError}
                  onChange={(e) => {
                    setTranslateStatus(null);
                    update({ translate_gemini_key: e.target.value });
                  }}
                />
                <FieldError>{translateGeminiKeyError}</FieldError>
                {!translateGeminiKeyError && (
                  <p className="text-[12px] text-muted mt-1.5">
                    Used only for translating. Stored separately from the transcription key.
                  </p>
                )}
              </div>
            )}
          </>
        ) : (
          <>
            <div>
              <Label htmlFor="translate-url">API URL</Label>
              <Input
                id="translate-url"
                placeholder="https://api.openai.com/v1"
                value={config.translate_base_url}
                invalid={!!translateUrlError}
                onChange={(e) => {
                  // Editing any of the three fields invalidates the last check,
                  // or a pass for the old endpoint sits there reading as a pass
                  // for the one just typed.
                  setTranslateStatus(null);
                  update({ translate_base_url: e.target.value });
                }}
              />
              <FieldError>{translateUrlError}</FieldError>
              {!translateUrlError && (
                <p className="text-[12px] text-muted mt-1.5">
                  The base URL, ending at <code>/v1</code> or whatever your service uses.{' '}
                  <code>/chat/completions</code> is added for you.
                </p>
              )}
            </div>

            <ModelField
              id="translate-custom-model"
              value={config.translate_custom_model}
              placeholder="gpt-4o-mini"
              url={config.translate_base_url}
              apiKey={config.translate_api_key}
              error={translateModelError}
              help={
                <>
                  Named exactly as your service expects it, e.g. <code>gpt-4o-mini</code> or{' '}
                  <code>llama3.1:8b</code>. Fetch models lists what the endpoint already has — the
                  models installed in your Ollama, for one.
                </>
              }
              onChange={(translate_custom_model) => {
                setTranslateStatus(null);
                update({ translate_custom_model });
              }}
            />

            <div>
              <Label htmlFor="translate-key">API key</Label>
              <Input
                id="translate-key"
                type="password"
                placeholder="Not needed for a local server"
                value={config.translate_api_key}
                onChange={(e) => {
                  setTranslateStatus(null);
                  update({ translate_api_key: e.target.value });
                }}
              />
            </div>
          </>
        )}

        {/* Shown for either service: translating is the one thing here that can
            be configured and still not work, and a Gemini key used only for
            translating has no other way to be checked — the button in the
            transcription card only ever tests the provider that transcribes. */}
        <div className="flex items-center gap-3">
          <Button
            loading={translateChecking}
            loadingText="Testing…"
            status={translateStatus ? 'success' : 'idle'}
            statusText="Working"
            disabled={!!translateUrlError || !!translateModelError || !!translateGeminiKeyError}
            onClick={async () => {
              setTranslateChecking(true);
              try {
                // The fields above save on a debounce, so a check clicked
                // straight after typing would otherwise test the values from
                // before. An empty patch writes what is on screen now.
                await updateNow({});
                const message = await api.verifyTranslate();
                setTranslateStatus(message);
                notify('Translation is working', 'ok');
              } catch (e) {
                // As above: the card carries it, and the summary here would be
                // the static "Checking the translation service".
                setTranslateStatus(null);
                fail(api.problemOf(e));
              } finally {
                setTranslateChecking(false);
              }
            }}
          >
            <ShieldCheck className="w-4 h-4" />
            Test translation
          </Button>
          <span className="text-[12px] text-muted">
            {translateStatus ??
              'Translates one short phrase, so a wrong key, model or URL is found here.'}
          </span>
        </div>

        <p className="text-[12px] text-muted">
          Translation is one extra round trip after you finish speaking, which is why it has a key
          of its own rather than being something every dictation waits for. If it fails, the words
          you actually said are typed instead, and the reason is shown.
        </p>
      </Card>

      <SectionTitle>Quitting</SectionTitle>
      <Card>
        <Row
          label="Quit Orra"
          sub="Closing this window keeps dictation running in the background. This stops it."
        >
          {quitting ? (
            <div className="flex gap-2">
              <Button
                variant="danger"
                loading={quitBusy}
                loadingText="Quitting…"
                onClick={async () => {
                  setQuitBusy(true);
                  try {
                    await api.quit();
                  } catch (e) {
                    fail(api.problemOf(e));
                    setQuitBusy(false);
                  }
                }}
              >
                Quit now
              </Button>
              <Button onClick={() => setQuitting(false)}>Cancel</Button>
            </div>
          ) : (
            <Button variant="danger" onClick={() => setQuitting(true)}>
              <LogOut className="w-4 h-4" />
              Quit
            </Button>
          )}
        </Row>
        <p className="text-[12px] text-muted mt-3">
          Version {status?.version ?? '—'}
          {status?.hyprland ? ' · shortcuts are owned by Hyprland' : ''}
        </p>
      </Card>
    </>
  );
}
