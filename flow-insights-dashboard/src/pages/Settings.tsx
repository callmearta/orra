import { useState } from 'react';
import { LogOut, RotateCw, ShieldCheck } from 'lucide-react';

import { HotkeyInput } from '@/components/HotkeyInput';
import {
  Button,
  Card,
  Divider,
  Input,
  Label,
  PageHeading,
  Pill,
  Row,
  SectionTitle,
  Select,
  Toggle,
} from '@/components/ui';
import * as api from '@/lib/api';
import { TRANSLATE_LANGUAGES, TRANSLATE_MODELS } from '@/lib/translate-catalog';
import { useStore } from '@/store';

/**
 * What each provider is, in one line — including which model it will use, since
 * two of them pick that themselves and nothing else in the interface says so.
 */
const PROVIDER_BLURB: Record<api.Provider, string> = {
  deepgram: 'nova-3 and the other Deepgram models. Its model, language and keyterms are on the Voice page.',
  assemblyai: 'AssemblyAI hears the audio itself and picks its own model, so the Deepgram settings on the Voice page do not apply.',
  gemini: 'Streams with gemini-3.5-transcribe-live. It detects the language itself and takes no keyterms, so the Deepgram settings on the Voice page do not apply.',
};

export default function SettingsPage() {
  const { config, status, notify, fail, update, updateNow } = useStore();
  const [keyStatus, setKeyStatus] = useState<string | null>(null);
  const [translateStatus, setTranslateStatus] = useState<string | null>(null);
  const [quitting, setQuitting] = useState(false);

  if (!config) return null;

  const mode = config.mode;

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
            onChange={(hotkey) => void updateNow({ hotkey })}
          />
          <p className="text-[12px] text-muted mt-1.5">
            Click the box, then press the combination you want.
          </p>
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

        {/* The language key steps Deepgram's language cycle; no other provider
            has one to step. */}
        {config.provider === 'deepgram' && (
          <div>
            <Label htmlFor="language-hotkey">Switch language key</Label>
            <HotkeyInput
              label="Switch language key"
              value={config.language_hotkey}
              onChange={(language_hotkey) => void updateNow({ language_hotkey })}
            />
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
            onClick={async () => {
              try {
                notify(await api.applyHotkey(), 'ok');
              } catch (e) {
                fail(api.problemOf(e));
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
              // The last check was about the provider being switched away from.
              setKeyStatus(null);
              void updateNow({ provider: e.target.value as api.Provider });
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

        <div>
          <Label htmlFor="provider-key">{api.providerLabel(config.provider)} API key</Label>
          <Input
            id="provider-key"
            type="password"
            placeholder="Read from .env — paste here to override"
            value={config[api.KEY_FIELD[config.provider]]}
            onChange={(e) => update({ [api.KEY_FIELD[config.provider]]: e.target.value })}
          />
          <p className="text-[12px] text-muted mt-1.5">
            Resolution order is environment, then the nearest <code>.env</code>, then this value.
            Read aloud always uses Deepgram's voices, so it needs a Deepgram key too.
          </p>
        </div>

        <div className="flex items-center gap-3">
          <Button
            onClick={async () => {
              setKeyStatus('Checking…');
              try {
                const message = await api.verifyKey();
                setKeyStatus(message);
                notify(`${api.providerLabel(config.provider)} key verified`, 'ok');
              } catch (e) {
                // Cleared rather than filled in: the card above carries the
                // whole failure, and this line would only repeat its summary —
                // which for this command is the static "Checking the … key",
                // reading as though the check were still running.
                setKeyStatus(null);
                fail(api.problemOf(e));
              }
            }}
          >
            <ShieldCheck className="w-4 h-4" />
            Verify key
          </Button>
          <span className="text-[12px] text-muted">
            {keyStatus ?? (status?.has_key ? 'A key is available.' : 'No key found.')}
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
              ? 'Whichever provider is transcribing, translating runs on Gemini, so it uses the Gemini key.'
              : 'Any service that speaks the OpenAI /chat/completions API: OpenAI, OpenRouter, Groq, or a model running on this machine.'}
          </p>
        </div>

        {config.translate_provider === 'gemini' ? (
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
        ) : (
          <>
            <div>
              <Label htmlFor="translate-url">API URL</Label>
              <Input
                id="translate-url"
                placeholder="https://api.openai.com/v1"
                value={config.translate_base_url}
                onChange={(e) => {
                  // Editing any of the three fields invalidates the last check,
                  // or a pass for the old endpoint sits there reading as a pass
                  // for the one just typed.
                  setTranslateStatus(null);
                  update({ translate_base_url: e.target.value });
                }}
              />
              <p className="text-[12px] text-muted mt-1.5">
                The base URL, ending at <code>/v1</code> or whatever your service uses.{' '}
                <code>/chat/completions</code> is added for you.
              </p>
            </div>

            <div>
              <Label htmlFor="translate-custom-model">Model</Label>
              <Input
                id="translate-custom-model"
                placeholder="gpt-4o-mini"
                value={config.translate_custom_model}
                onChange={(e) => {
                  setTranslateStatus(null);
                  update({ translate_custom_model: e.target.value });
                }}
              />
              <p className="text-[12px] text-muted mt-1.5">
                Named exactly as your service expects it, e.g. <code>gpt-4o-mini</code> or{' '}
                <code>llama3.1:8b</code>.
              </p>
            </div>

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
            onClick={async () => {
              setTranslateStatus('Translating a test phrase…');
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
              <Button variant="danger" onClick={() => void api.quit()}>
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
