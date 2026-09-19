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
  const { config, status, notify, update, updateNow } = useStore();
  const [keyStatus, setKeyStatus] = useState<string | null>(null);
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
                notify(api.errorText(e), 'err');
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
                setKeyStatus(api.errorText(e));
                notify(api.errorText(e), 'err');
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

        <Row
          label="Model"
          sub="Translation runs on Gemini, whichever provider is transcribing, so it uses the Gemini key."
        >
          <Select
            className="w-auto"
            aria-label="Translation model"
            value={config.translate_model}
            onChange={(e) => update({ translate_model: e.target.value })}
          >
            {TRANSLATE_MODELS.map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </Select>
        </Row>

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
