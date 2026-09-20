import { useState } from 'react';
import { ClipboardCopy, CornerDownLeft, Trash2, Volume2 } from 'lucide-react';

import { Button, Card, Divider, Empty, PageHeading, Pill, SectionTitle } from '@/components/ui';
import * as api from '@/lib/api';
import { duration, num, prettifyApp, timeAgo } from '@/lib/stats';
import { useStore } from '@/store';

export default function TranscriptsPage() {
  const { history, notify, fail, refreshHistory } = useStore();
  const [confirmingClear, setConfirmingClear] = useState(false);

  const totalWords = history.reduce((n, e) => n + e.words, 0);

  const run = async (what: string, action: () => Promise<unknown>) => {
    try {
      await action();
      if (what) notify(what, 'ok');
    } catch (e) {
      fail(api.problemOf(e));
    }
  };

  return (
    <>
      <PageHeading
        title="Transcripts & Notes"
        lede="Everything you have dictated, newest first. Insert puts a transcript back into whatever window is focused now."
        aside={
          history.length > 0 ? (
            <Pill>
              {num(history.length)} · {num(totalWords)} words
            </Pill>
          ) : undefined
        }
      />

      <SectionTitle
        aside={confirmingClear ? 'Are you sure?' : 'Capped at 5,000 entries'}
      >
        Recent transcripts
      </SectionTitle>

      <Card>
        <div className="flex items-center justify-between gap-4">
          <div className="text-[12px] text-muted">
            Kept locally in <code>~/.config/orra/history.jsonl</code>.
          </div>
          {confirmingClear ? (
            <div className="flex gap-2">
              <Button
                variant="danger"
                onClick={() =>
                  run('History cleared', async () => {
                    await api.clearHistory();
                    await refreshHistory();
                    setConfirmingClear(false);
                  })
                }
              >
                Delete all {num(history.length)}
              </Button>
              <Button onClick={() => setConfirmingClear(false)}>Cancel</Button>
            </div>
          ) : (
            <Button
              variant="danger"
              disabled={history.length === 0}
              onClick={() => setConfirmingClear(true)}
            >
              Clear all
            </Button>
          )}
        </div>
      </Card>

      <div className="mt-4 flex flex-col gap-3">
        {history.length === 0 && (
          <Card>
            <Empty>No dictations yet. Hold your key and speak.</Empty>
          </Card>
        )}

        {history.map((entry) => (
          <Card key={entry.id} className="p-5 sm:p-6">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[12px] text-muted">
              <span className="font-semibold text-ink">{timeAgo(entry.at)}</span>
              {entry.app && <span title={entry.app}>{prettifyApp(entry.app)}</span>}
              <span>·</span>
              <span>
                {num(entry.words)} {entry.words === 1 ? 'word' : 'words'}
              </span>
              {/* Only shown where a duration was actually recorded. */}
              {entry.ms ? (
                <>
                  <span>·</span>
                  <span>{duration(entry.ms)}</span>
                </>
              ) : null}
              {(entry.fixes > 0 || entry.fillers > 0) && (
                <>
                  <span>·</span>
                  <span>
                    {entry.fixes > 0 && `${entry.fixes} replaced`}
                    {entry.fixes > 0 && entry.fillers > 0 && ', '}
                    {entry.fillers > 0 && `${entry.fillers} fillers cut`}
                  </span>
                </>
              )}
            </div>

            <Divider />

            <p className="text-[15px] leading-relaxed whitespace-pre-wrap">{entry.text}</p>

            {/* A translated dictation keeps the words that were spoken, which
                are otherwise nowhere once the translation has been typed. */}
            {entry.source && (
              <p className="text-[13px] text-muted mt-2 whitespace-pre-wrap">
                Said: {entry.source}
              </p>
            )}

            <div className="flex flex-wrap gap-2 mt-4">
              <Button
                variant="mini"
                onClick={() =>
                  run('Copied', () => navigator.clipboard.writeText(entry.text))
                }
              >
                <ClipboardCopy className="w-3.5 h-3.5" />
                Copy
              </Button>
              <Button
                variant="mini"
                onClick={() =>
                  run('Inserted into the focused window', () => api.reinject(entry.id))
                }
              >
                <CornerDownLeft className="w-3.5 h-3.5" />
                Insert
              </Button>
              <Button variant="mini" onClick={() => run('', () => api.speak(entry.text))}>
                <Volume2 className="w-3.5 h-3.5" />
                Read aloud
              </Button>
              <Button
                variant="mini"
                onClick={() =>
                  run('', async () => {
                    await api.deleteHistory(entry.id);
                    await refreshHistory();
                  })
                }
              >
                <Trash2 className="w-3.5 h-3.5" />
                Delete
              </Button>
            </div>
          </Card>
        ))}
      </div>
    </>
  );
}
