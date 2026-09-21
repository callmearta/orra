import { useState, type ReactNode } from 'react';
import { List } from 'lucide-react';

import { Button, Input, Label } from '@/components/ui';
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
 * A `<datalist>` rather than a `<select>`: it is a dropdown and free text in one
 * control, which is what these fields need, since some of these servers publish
 * no list at all — whisper.cpp's own endpoint among them — and the model may be
 * named something the list does not mention.
 */
export function ModelField({
  id,
  value,
  placeholder,
  url,
  apiKey,
  help,
  onChange,
}: {
  id: string;
  value: string;
  placeholder: string;
  /** The endpoint to ask. Whatever is on screen, not what was last saved. */
  url: string;
  apiKey: string;
  help: ReactNode;
  onChange: (model: string) => void;
}) {
  const { fail } = useStore();
  const [models, setModels] = useState<string[]>([]);
  const [note, setNote] = useState<string | null>(null);

  return (
    <div>
      <div className="flex items-end gap-3">
        <div className="flex-1">
          <Label htmlFor={id}>Model</Label>
          <Input
            id={id}
            list={`${id}-models`}
            placeholder={placeholder}
            value={value}
            onChange={(e) => {
              // Whatever was fetched was for the old endpoint, and a count
              // sitting under a field that has just been edited reads as a fact
              // about the new one.
              setNote(null);
              onChange(e.target.value);
            }}
          />
          <datalist id={`${id}-models`}>
            {models.map((model) => (
              <option key={model} value={model} />
            ))}
          </datalist>
        </div>
        <Button
          onClick={async () => {
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
            }
          }}
        >
          <List className="w-4 h-4" />
          Fetch models
        </Button>
      </div>
      <p className="text-[12px] text-muted mt-1.5">{note ?? help}</p>
    </div>
  );
}
