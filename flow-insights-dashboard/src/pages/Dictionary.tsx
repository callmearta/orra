import { Plus, Trash2 } from 'lucide-react';

import { Button, Card, Divider, Empty, Input, PageHeading, Row, SectionTitle, Toggle } from '@/components/ui';
import * as api from '@/lib/api';
import { num } from '@/lib/stats';
import { useStore } from '@/store';

/**
 * Keyterms sent to Deepgram, plus the cleanup switches that decide what happens
 * to the transcript on the way back.
 */
export default function DictionaryPage() {
  const { config, update } = useStore();
  if (!config) return null;

  const words = config.dictionary;

  const setWords = (next: string[]) => update({ dictionary: next });

  return (
    <>
      <PageHeading
        title="Dictionary & Formatting"
        lede="Teach Orra your names and jargon, and choose how it tidies what it hears."
        aside={words.length > 0 ? <span>{num(words.length)} words</span> : undefined}
      />

      <SectionTitle aside="Sent as keyterms with every request">Dictionary</SectionTitle>

      {config.provider !== 'deepgram' && (
        <p className="text-[12px] text-muted mb-3">
          Keyterms are a Deepgram feature, so nothing here reaches{' '}
          {api.providerLabel(config.provider)}. The replacements below still apply to every
          dictation.
        </p>
      )}

      <Card>
        {words.length === 0 ? (
          <Empty>
            No words yet. Add names, products and acronyms so the model expects them.
          </Empty>
        ) : (
          <div className="flex flex-col">
            {words.map((word, index) => (
              <div key={index}>
                {index > 0 && <Divider />}
                <div className="flex items-center gap-3">
                  <Input
                    aria-label={`Dictionary word ${index + 1}`}
                    placeholder="Orra, Kubernetes, your surname…"
                    value={word}
                    onChange={(e) => {
                      const next = [...words];
                      next[index] = e.target.value;
                      setWords(next);
                    }}
                  />
                  <Button
                    variant="mini"
                    className="shrink-0"
                    aria-label={`Remove word ${index + 1}`}
                    onClick={() => setWords(words.filter((_, i) => i !== index))}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="mt-4">
          <Button onClick={() => setWords([...words, ''])}>
            <Plus className="w-4 h-4" />
            Add word
          </Button>
        </div>

        <p className="text-[12px] text-muted mt-4">
          nova-3 and flux take these as <code>keyterm</code>; older models take weighted{' '}
          <code>keywords</code>. Empty entries are skipped.
        </p>
      </Card>

      <SectionTitle>Formatting</SectionTitle>
      <Card className="flex flex-col gap-4">
        <Row
          label="Smart formatting"
          sub="Punctuation, capitalisation and numbers the way you would write them."
        >
          <Toggle
            label="Smart formatting"
            checked={config.smart_format}
            onChange={(smart_format) => update({ smart_format })}
          />
        </Row>
        <Divider />
        <Row
          label="Voice commands"
          sub="Understands new line, new paragraph, press enter and scratch that."
        >
          <Toggle
            label="Voice commands"
            checked={config.voice_commands}
            onChange={(voice_commands) => update({ voice_commands })}
          />
        </Row>
        <Divider />
        <Row
          label="Remove filler words"
          sub="Drops um, uh, hmm and friends — and stops counting them as cut."
        >
          <Toggle
            label="Remove filler words"
            checked={config.remove_fillers}
            onChange={(remove_fillers) => update({ remove_fillers })}
          />
        </Row>
      </Card>
    </>
  );
}
