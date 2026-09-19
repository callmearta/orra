import { Plus, Trash2 } from 'lucide-react';

import { Button, Card, Divider, Empty, Input, PageHeading, SectionTitle } from '@/components/ui';
import type { Rule } from '@/lib/api';
import { num } from '@/lib/stats';
import { useStore } from '@/store';

/**
 * The replacement rules, framed as the dashboard frames them: snippets and
 * shortcuts. A rule runs after transcription, case-insensitively and on word
 * boundaries, so it both fixes recurring mishearings and expands shorthand.
 */
export default function SnippetsPage() {
  const { config, update } = useStore();
  if (!config) return null;

  const rules = config.replacements;

  const write = (next: Rule[]) => update({ replacements: next });

  return (
    <>
      <PageHeading
        title="Snippets & Shortcuts"
        lede={
          <>
            Applied after transcription, case-insensitively and only on whole words. Fixes a
            recurring mishearing, or turns spoken shorthand into a paragraph — say{' '}
            <em>my signature</em>, get your full sign-off.
          </>
        }
        aside={rules.length > 0 ? <span>{num(rules.length)} rules</span> : undefined}
      />

      <SectionTitle aside="Spoken → written">Rules</SectionTitle>

      <Card>
        {rules.length === 0 ? (
          <Empty>
            No snippets yet. “or ra” → “Orra” fixes a mishearing; “my signature” → your
            sign-off expands one.
          </Empty>
        ) : (
          <div className="flex flex-col">
            {rules.map((rule, index) => (
              <div key={index}>
                {index > 0 && <Divider />}
                <div className="flex items-center gap-3">
                  <Input
                    aria-label={`Heard as, rule ${index + 1}`}
                    placeholder="heard as…"
                    value={rule.from}
                    onChange={(e) => {
                      const next = [...rules];
                      next[index] = { ...rule, from: e.target.value };
                      write(next);
                    }}
                  />
                  <span className="text-muted shrink-0">→</span>
                  {/* The replacement is free text, so newlines are allowed. */}
                  <Input
                    aria-label={`Replace with, rule ${index + 1}`}
                    placeholder="replace with…"
                    value={rule.to}
                    onChange={(e) => {
                      const next = [...rules];
                      next[index] = { ...rule, to: e.target.value };
                      write(next);
                    }}
                  />
                  <Button
                    variant="mini"
                    aria-label={`Remove rule ${index + 1}`}
                    className="shrink-0"
                    onClick={() => write(rules.filter((_, i) => i !== index))}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="mt-4">
          <Button onClick={() => write([...rules, { from: '', to: '' }])}>
            <Plus className="w-4 h-4" />
            Add snippet
          </Button>
        </div>
      </Card>

      <SectionTitle>Built-in voice commands</SectionTitle>
      <Card>
        <p className="text-[13px] text-muted">
          These run before your rules and are toggled under Dictionary &amp; Formatting:{' '}
          <code>new line</code>, <code>new paragraph</code>, <code>press enter</code>,{' '}
          <code>scratch that</code>, <code>question mark</code>, <code>exclamation mark</code>,{' '}
          <code>full stop</code>, <code>open paren</code>, <code>close paren</code>,{' '}
          <code>semicolon</code>, <code>hyphen</code>.
        </p>
        <p className="text-[13px] text-muted mt-3">
          <code>period</code>, <code>comma</code>, <code>colon</code> and <code>dash</code> are
          deliberately absent — they are ordinary nouns, and rewriting them mid-sentence does more
          harm than good. Add them here if you want them.
        </p>
      </Card>
    </>
  );
}
