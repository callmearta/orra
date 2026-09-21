/**
 * The Deepgram model, language and voice tables.
 *
 * A curated catalogue, not something derived from the config — the descriptions
 * are the ones worth showing, and the provider does not publish a list.
 */

export const STT_MODELS: [string, string][] = [
  ['nova-3', 'Nova 3 — best accuracy'],
  ['nova-3-medical', 'Nova 3 Medical'],
  ['nova-2', 'Nova 2 — faster, cheaper'],
  ['nova-2-phonecall', 'Nova 2 Phonecall'],
  ['nova-2-meeting', 'Nova 2 Meeting'],
  ['nova-2-conversationalai', 'Nova 2 Conversational AI'],
  ['enhanced', 'Enhanced'],
  ['base', 'Base — cheapest'],
];

/**
 * `multi` is nova-3's multilingual streaming mode, not a language. Deepgram has
 * no auto-detect on the streaming endpoint, so this is the closest equivalent.
 */
export const LANGUAGES: [string, string][] = [
  ['multi', 'Multilingual (nova-3)'],
  ['en', 'English'], ['en-US', 'English (US)'], ['en-GB', 'English (UK)'],
  ['en-AU', 'English (Australia)'], ['en-IN', 'English (India)'],
  ['fa', 'Persian (Farsi)'], ['ar', 'Arabic'], ['he', 'Hebrew'], ['tr', 'Turkish'],
  ['ur', 'Urdu'], ['ps', 'Pashto'], ['ku', 'Kurdish'],
  ['es', 'Spanish'], ['fr', 'French'], ['de', 'German'], ['it', 'Italian'],
  ['nl', 'Dutch'], ['pt', 'Portuguese'], ['ru', 'Russian'], ['uk', 'Ukrainian'],
  ['pl', 'Polish'], ['cs', 'Czech'], ['ro', 'Romanian'], ['el', 'Greek'],
  ['sv', 'Swedish'], ['da', 'Danish'], ['no', 'Norwegian'], ['fi', 'Finnish'],
  ['hi', 'Hindi'], ['bn', 'Bengali'], ['ta', 'Tamil'], ['te', 'Telugu'],
  ['mr', 'Marathi'], ['gu', 'Gujarati'], ['pa', 'Punjabi'], ['ne', 'Nepali'],
  ['ja', 'Japanese'], ['ko', 'Korean'], ['zh', 'Chinese'],
  ['zh-CN', 'Chinese (Simplified)'], ['zh-TW', 'Chinese (Traditional)'],
  ['vi', 'Vietnamese'], ['th', 'Thai'], ['id', 'Indonesian'], ['ms', 'Malay'],
  ['tl', 'Filipino'], ['hy', 'Armenian'], ['ka', 'Georgian'], ['az', 'Azerbaijani'],
  ['kk', 'Kazakh'], ['uz', 'Uzbek'], ['af', 'Afrikaans'], ['sw', 'Swahili'],
];

export const TTS_VOICES: [string, string][] = [
  ['aura-2-thalia-en', 'Thalia — warm, clear'], ['aura-2-andromeda-en', 'Andromeda — bright'],
  ['aura-2-helena-en', 'Helena — calm'], ['aura-2-apollo-en', 'Apollo — confident'],
  ['aura-2-arcas-en', 'Arcas — even'], ['aura-2-aries-en', 'Aries — energetic'],
  ['aura-2-atlas-en', 'Atlas — deep'], ['aura-2-aurora-en', 'Aurora — friendly'],
  ['aura-2-callista-en', 'Callista — smooth'], ['aura-2-cora-en', 'Cora — gentle'],
  ['aura-2-cordelia-en', 'Cordelia — expressive'], ['aura-2-delia-en', 'Delia — casual'],
  ['aura-2-draco-en', 'Draco — authoritative'], ['aura-2-electra-en', 'Electra — crisp'],
  ['aura-2-harmonia-en', 'Harmonia — soothing'], ['aura-2-hera-en', 'Hera — poised'],
  ['aura-2-hermes-en', 'Hermes — lively'], ['aura-2-hyperion-en', 'Hyperion — resonant'],
  ['aura-2-iris-en', 'Iris — upbeat'], ['aura-2-janus-en', 'Janus — measured'],
  ['aura-2-juno-en', 'Juno — polished'], ['aura-2-luna-en', 'Luna — soft'],
  ['aura-2-minerva-en', 'Minerva — articulate'], ['aura-2-neptune-en', 'Neptune — steady'],
  ['aura-2-odysseus-en', 'Odysseus — narrative'], ['aura-2-ophelia-en', 'Ophelia — lyrical'],
  ['aura-2-orion-en', 'Orion — grounded'], ['aura-2-orpheus-en', 'Orpheus — mellow'],
  ['aura-2-pandora-en', 'Pandora — playful'], ['aura-2-phoebe-en', 'Phoebe — light'],
  ['aura-2-pluto-en', 'Pluto — sombre'], ['aura-2-saturn-en', 'Saturn — deliberate'],
  ['aura-2-selene-en', 'Selene — dreamy'], ['aura-2-theia-en', 'Theia — radiant'],
  ['aura-2-vesta-en', 'Vesta — practical'], ['aura-2-zeus-en', 'Zeus — commanding'],
];

/** Label for a language code, for the status line and the overlay. */
export function languageLabel(code: string): string {
  const known = LANGUAGES.find(([c]) => c === code);
  if (known) return known[1];
  if (code === 'multi') return 'Multilingual';
  return code.toUpperCase();
}
