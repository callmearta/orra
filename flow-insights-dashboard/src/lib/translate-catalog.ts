/**
 * What the translation pickers offer.
 *
 * The languages are the same codes the rest of the app uses, minus `multi`,
 * which is Deepgram's word for "no language chosen" and means nothing as a
 * thing to translate *into*.
 */
import { LANGUAGES } from '@/lib/deepgram-catalog';

export const TRANSLATE_LANGUAGES: [string, string][] = LANGUAGES.filter(
  ([code]) => code !== 'multi',
);

/**
 * The Gemini models offered for translating. A flash model is plenty — the
 * work is one short instruction and a paragraph of text — and the newest one is
 * not automatically the best choice, since it is also the busiest.
 */
export const TRANSLATE_MODELS: [string, string][] = [
  ['gemini-3.5-flash', 'Gemini 3.5 Flash'],
  ['gemini-3.5-flash-lite', 'Gemini 3.5 Flash Lite — cheapest'],
  ['gemini-3.1-flash-lite', 'Gemini 3.1 Flash Lite'],
  ['gemini-2.5-flash', 'Gemini 2.5 Flash'],
  ['gemini-3.8-flash', 'Gemini 3.8 Flash — newest, busiest'],
];

/**
 * What AssemblyAI's streaming socket will steer towards — eighteen languages.
 *
 * Persian is not among them: AssemblyAI only transcribes it through its
 * pre-recorded endpoint, so offering it here would promise something the
 * socket silently ignores.
 */
export const ASSEMBLYAI_LANGUAGES: [string, string][] = [
  ['en', 'English'], ['ar', 'Arabic'], ['zh', 'Chinese'], ['da', 'Danish'],
  ['nl', 'Dutch'], ['fi', 'Finnish'], ['fr', 'French'], ['de', 'German'],
  ['he', 'Hebrew'], ['hi', 'Hindi'], ['it', 'Italian'], ['ja', 'Japanese'],
  ['no', 'Norwegian'], ['pt', 'Portuguese'], ['es', 'Spanish'], ['sv', 'Swedish'],
  ['tr', 'Turkish'], ['vi', 'Vietnamese'],
];
