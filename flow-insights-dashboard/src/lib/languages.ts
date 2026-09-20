import type { Provider } from '@/lib/api';
import { LANGUAGES, languageLabel } from '@/lib/deepgram-catalog';
import { ASSEMBLYAI_LANGUAGES } from '@/lib/translate-catalog';

/** A dictation language: the code that is sent, and what to call it. */
export type Language = [code: string, label: string];

/**
 * The languages a provider can be steered towards, or none.
 *
 * One list per provider because they are genuinely different lists: Deepgram
 * takes about fifty codes and has a multilingual mode, AssemblyAI steers
 * eighteen and silently ignores the rest, and a whisper server detects the
 * language for itself unless it is given one. Gemini takes no code at all,
 * which is what an empty list means — and what keeps a language control off its
 * pages rather than offering one that does nothing.
 */
export function languagesFor(provider: Provider): Language[] {
  switch (provider) {
    case 'gemini':
      return [];
    case 'assemblyai':
      // An empty code is how this one is told to steer nowhere.
      return [['', 'Automatic — no steering'], ...ASSEMBLYAI_LANGUAGES];
    case 'deepgram':
      return LANGUAGES;
    default:
      // A server the user runs, or the model Orra runs. `multi` is Deepgram's
      // word for following the speaker, and leaving the field out of the
      // request is how these are told the same thing.
      return [
        ['multi', 'Detect automatically'],
        ...LANGUAGES.filter(([code]) => code !== 'multi'),
      ];
  }
}

/** What to call the configured language, whether or not it is in the list. */
export function languageName(code: string, options: Language[]): string {
  return options.find(([value]) => value === code)?.[1] ?? languageLabel(code);
}
