import { useState, type ReactNode } from 'react';
import { Check, ChevronDown, List } from 'lucide-react';

import { Button, FieldError, Input, Label } from '@/components/ui';
import * as api from '@/lib/api';
import { useStore } from '@/store';

/**
 * A model field that can ask the endpoint what it has.
 *
 * Used everywhere a model name is configured against an OpenAI-compatible
 * service — the server transcribing and the one translating are the same kind
 * of thing — because both want the list of what is installed rather than a name
 * typed from memory.
 *
 * A custom combobox rather than a native `<select>` or `<datalist>`: it is a
 * dropdown and free text in one control, which is what these fields need, since
 * some of these servers publish no list at all — whisper.cpp's own endpoint
 * among them — and the model may be named something the list does not mention.
 */
export function ModelField({
  id,
  value,
  placeholder,
  url,
  apiKey,
  help,
  error,
  onChange,
}: {
  id: string;
  value: string;
  placeholder: string;
  error?: string | null;
  /** The endpoint to ask. Whatever is on screen, not what was last saved. */
  url: string;
  apiKey: string;
  help: ReactNode;
  onChange: (model: string) => void;
}) {
  const { fail } = useStore();
  const [models, setModels] = useState<string[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [open, setOpen] = useState(false);
  const filtered = models.filter((model) => model.toLowerCase().includes(value.trim().toLowerCase()));

  return (
    <div>
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <Label htmlFor={id}>Model</Label>
          <div className="relative">
            <Input
              id={id}
              role="combobox"
              aria-expanded={open}
              aria-controls={`${id}-models`}
              aria-autocomplete="list"
              placeholder={placeholder}
              value={value}
              invalid={!!error}
              onFocus={() => setOpen(true)}
              onBlur={() => window.setTimeout(() => setOpen(false), 120)}
              onChange={(e) => {
                // Whatever was fetched was for the old endpoint, and a count
                // sitting under a field that has just been edited reads as a fact
                // about the new one.
                setNote(null);
                setOpen(true);
                onChange(e.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') setOpen(false);
              }}
              className="pr-9"
            />
            <button
              type="button"
              tabIndex={-1}
              aria-label="Show fetched models"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => setOpen((current) => !current)}
              className="absolute right-2 top-1/2 -translate-y-1/2 p-1 rounded-lg text-muted hover:text-ink hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer"
            >
              <ChevronDown className={`w-4 h-4 transition-transform ${open ? 'rotate-180' : ''}`} />
            </button>
            {open && filtered.length > 0 && (
              <div
                id={`${id}-models`}
                role="listbox"
                className="absolute left-0 right-0 top-full z-50 mt-1.5 max-h-56 overflow-y-auto rounded-2xl border border-hair bg-sheet p-1.5 shadow-2xl shadow-black/15"
                onMouseDown={(event) => event.preventDefault()}
              >
                {filtered.map((model) => (
                  <button
                    key={model}
                    type="button"
                    role="option"
                    aria-selected={model === value}
                    onClick={() => {
                      onChange(model);
                      setOpen(false);
                    }}
                    className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-[13px] hover:bg-black/5 dark:hover:bg-white/10 cursor-pointer"
                  >
                    <Check className={`w-3.5 h-3.5 shrink-0 ${model === value ? 'opacity-100' : 'opacity-0'}`} />
                    <span className="truncate">{model}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
        <Button
          loading={loading}
          loadingText="Fetching…"
          status={!loading && note && !note.startsWith('Asking') ? 'success' : 'idle'}
          statusText="Fetched"
          onClick={async () => {
            setLoading(true);
            setNote('Asking the server…');
            try {
              const found = await api.listModels(url, apiKey);
              setModels(found);
              setNote(`${found.length} model${found.length === 1 ? '' : 's'} on that server.`);
            } catch (e) {
              // The problem card carries the failure, which names the URL and
              // quotes what the endpoint said about it.
              setModels([]);
              setNote(null);
              fail(api.problemOf(e));
            } finally {
              setLoading(false);
            }
          }}
        >
          <List className="w-4 h-4" />
          Fetch models
        </Button>
      </div>
      <FieldError>{error}</FieldError>
      {!error && <p className="text-[12px] text-muted mt-1.5">{note ?? help}</p>}
    </div>
  );
}
