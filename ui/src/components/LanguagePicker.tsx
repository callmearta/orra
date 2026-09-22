import { useRef } from 'react';
import { Check, Languages } from 'lucide-react';

import { Button } from '@/components/ui';
import { languageName, type Language } from '@/lib/languages';

/**
 * The dictation language: a button that opens the list.
 *
 * A modal rather than a cycling button, because there is no order to cycle
 * through — the list is every language the provider takes, and picking one is
 * the whole interaction. A native `<dialog>` rather than a hand-rolled overlay:
 * it brings the backdrop, the focus trap and Escape with it, which is most of
 * what a modal is made of.
 */
export function LanguagePicker({
  current,
  options,
  note,
  loading = false,
  onPick,
}: {
  current: string;
  options: Language[];
  /** One line about what the choice means for this provider. */
  note: string;
  loading?: boolean;
  onPick: (code: string) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);

  return (
    <>
      <Button
        onClick={() => dialog.current?.showModal()}
        loading={loading}
        loadingText="Switching…"
      >
        <Languages className="w-4 h-4" />
        {languageName(current, options)}
      </Button>

      <dialog
        ref={dialog}
        // A click on the backdrop lands on the dialog element itself; one on
        // the panel lands on a child. That is the whole dismiss-on-outside.
        onClick={(e) => {
          if (e.target === dialog.current) dialog.current?.close();
        }}
        className="m-auto max-h-[85vh] w-[min(560px,92vw)] rounded-[22px] bg-sheet p-0 text-ink backdrop:bg-black/45"
      >
        <div className="flex flex-col gap-4 p-6">
          <div>
            <h2 className="text-[15px] font-bold tracking-tight">Dictation language</h2>
            <p className="text-[12px] text-muted mt-1">{note}</p>
          </div>

          <div className="grid grid-cols-2 sm:grid-cols-3 gap-1.5 overflow-y-auto max-h-[52vh] pr-1">
            {options.map(([code, label]) => {
              const selected = code === current;
              return (
                <button
                  key={code || 'auto'}
                  type="button"
                  onClick={() => {
                    onPick(code);
                    dialog.current?.close();
                  }}
                  className={`flex items-center gap-1.5 rounded-lg px-2.5 py-2 text-left text-[12px] transition-colors cursor-pointer ${
                    selected
                      ? 'bg-teal text-white font-semibold'
                      : 'hover:bg-black/5 dark:hover:bg-white/10'
                  }`}
                >
                  {selected && <Check className="w-3.5 h-3.5 shrink-0" />}
                  <span className="truncate">{label}</span>
                </button>
              );
            })}
          </div>

          <div className="flex justify-end">
            <Button onClick={() => dialog.current?.close()}>Cancel</Button>
          </div>
        </div>
      </dialog>
    </>
  );
}
