/**
 * The whole bridge to the Rust side.
 *
 * `withGlobalTauri` is on in tauri.conf.json, so `window.__TAURI__` is injected
 * into every window and no `@tauri-apps/*` package is needed. That keeps the
 * built frontend plain static files, which is what the Tauri asset loader wants.
 */

/** Mirrors `config::Config` in src-tauri/src/config.rs. */
export interface Rule {
  from: string;
  to: string;
}

export type Mode = 'hold' | 'toggle';
export type Injection = 'clipboard-paste' | 'type';
export type Provider =
  | 'deepgram'
  | 'assemblyai'
  | 'gemini'
  // The servers people run themselves, each named for the one it is. They share
  // the `local_*` settings and the same code; what differs is the URL, and
  // which of the APIs that server answers on.
  | 'ollama'
  | 'speaches'
  | 'localai'
  | 'whispercpp'
  // Anything else speaking those APIs: a URL the user typed.
  | 'local'
  // A model Orra fetches and runs on this machine itself.
  | 'orra';
/**
 * How the local provider reaches its server.
 *
 * `http` posts the recording when the key is released, `sse` streams the
 * transcript back as the server decodes it, and `websocket` is the only one of
 * the three that shows words while they are still being spoken.
 */
export type LocalTransport = 'http' | 'sse' | 'websocket';
/**
 * What translates, which is independent of what transcribes: dictating with
 * Deepgram and translating with something else is the normal case.
 */
export type TranslateProvider = 'gemini' | 'custom';

/** The providers on offer, in the order they are listed. */
export const PROVIDERS: [Provider, string][] = [
  ['deepgram', 'Deepgram'],
  ['assemblyai', 'AssemblyAI'],
  ['gemini', 'Gemini'],
  ['ollama', 'Ollama'],
  ['speaches', 'Speaches'],
  ['localai', 'LocalAI'],
  ['whispercpp', 'whisper.cpp'],
  ['local', 'Custom endpoint'],
  ['orra', 'Orra — open-source models'],
];

/**
 * The transports on offer: the value, the name, and what each one actually
 * gives back — the last of which is the difference the user is choosing
 * between, so it is spelled out where they choose it.
 */
export const LOCAL_TRANSPORTS: [LocalTransport, string, string][] = [
  ['http', 'HTTP', 'The transcript arrives when you release the key. Works with every server.'],
  ['sse', 'HTTP (streaming)', 'Words appear as the server decodes them, a moment after you release — still not while you speak.'],
  ['websocket', 'WebSocket', 'Words appear while you are still speaking. Needs a server that speaks the Realtime API.'],
];

export const providerLabel = (p: Provider): string =>
  PROVIDERS.find(([value]) => value === p)?.[1] ?? p;

/** A `Config` field that holds an API key. */
export type KeyField = 'api_key' | 'assemblyai_key' | 'gemini_key' | 'local_key';

/**
 * What the backend says about a provider — mirrors `config::ProviderInfo`.
 *
 * The rules about which providers have a preset URL, a model list to fetch or a
 * transport to choose live in Rust, next to the code they describe, and arrive
 * here with the status. Anything the interface believed separately would be a
 * second copy to keep in step.
 */
export interface ProviderInfo {
  value: Provider;
  label: string;
  self_hosted: boolean;
  key_field: KeyField;
  env_var: string;
  preset_url: string | null;
  has_model_list: boolean;
  has_transport_choice: boolean;
  /** Whether the dictation language applies, and the switch key with it. */
  has_language: boolean;
}

export interface Config {
  hotkey: string;
  mode: Mode;
  provider: Provider;
  api_key: string;
  assemblyai_key: string;
  gemini_key: string;
  local_key: string;
  stt_model: string;
  language: string;
  /** Where the local provider's server is. */
  local_base_url: string;
  /** The model to ask it for. Free text; the list comes from the server. */
  local_model: string;
  local_transport: LocalTransport;
  /**
   * The model Orra runs on this machine itself, by name from the catalogue.
   * Empty means the user runs their own server — which is also how a launch
   * knows whether there is an engine to start.
   */
  local_engine_model: string;
  language_cycle: string[];
  language_hotkey: string;
  mic: string;
  smart_format: boolean;
  dictionary: string[];
  replacements: Rule[];
  translate_hotkey: string;
  translate_language: string;
  translate_provider: TranslateProvider;
  translate_model: string;
  /** OpenAI-compatible endpoint; `/chat/completions` is appended to it. */
  translate_base_url: string;
  translate_custom_model: string;
  translate_api_key: string;
  tts_enabled: boolean;
  tts_model: string;
  tts_autoplay: boolean;
  injection: Injection;
  restore_clipboard: boolean;
  trailing_space: boolean;
  auto_submit: boolean;
  voice_commands: boolean;
  remove_fillers: boolean;
  hud: boolean;
  sounds: boolean;
  launch_at_login: boolean;
  dark_mode: boolean;
  port: number;
  token: string;
}

/** Mirrors `problem::Kind` in src-tauri/src/problem.rs. */
export type ProblemKind =
  | 'network'
  | 'auth'
  | 'quota'
  | 'request'
  | 'config'
  | 'local'
  | 'unknown';

/**
 * A failure, as the backend describes it.
 *
 * Mirrors `problem::Problem`. `detail` is the error exactly as it came back,
 * and `log` is that plus the version and platform, ready to paste into a bug
 * report — the backend builds it because the API keys have to be taken out
 * before the text leaves the process.
 */
export interface Problem {
  kind: ProblemKind;
  /** The headline: what kind of failure this was. */
  title: string;
  /** What was being attempted, in plain words. */
  summary: string;
  /** What to do about it. */
  advice: string;
  /** The failure itself, verbatim. */
  detail: string;
  /** The whole thing, for a report. */
  log: string;
}

/** Mirrors `commands::Status`. */
export interface Status {
  config: Config;
  providers: ProviderInfo[];
  recording: boolean;
  speaking: boolean;
  /** True when Hyprland owns the shortcut rather than the app. */
  hyprland: boolean;
  has_key: boolean;
  version: string;
}

/** Mirrors `state::Entry`. */
export interface Entry {
  id: string;
  text: string;
  /** Unix milliseconds, at the end of the dictation. */
  at: number;
  app: string;
  words: number;
  /** Absent on entries recorded before durations were tracked. */
  ms?: number | null;
  fixes: number;
  fillers: number;
  confidence?: number | null;
  /** The words that were spoken, when `text` is a translation of them. */
  source?: string | null;
}

/** Mirrors `stats::Totals`. */
export interface Totals {
  words: number;
  dictations: number;
  apps: number;
  wpm: number | null;
  measured: number;
  fixes: number;
  fillers: number;
  confidence: number | null;
  vocabulary: number;
  words_this_month: number;
  words_last_month: number;
  minutes_saved: number;
  books: number;
}

/** Mirrors `stats::DayCount`. */
export interface DayCount {
  /** Local calendar day, `YYYY-MM-DD`. */
  day: string;
  words: number;
  dictations: number;
}

/** Mirrors `stats::AppCount`. */
export interface AppCount {
  app: string;
  words: number;
  dictations: number;
}

/** Mirrors `stats::Insights`. */
export interface Insights {
  totals: Totals;
  days: DayCount[];
  apps: AppCount[];
  current_streak: number;
  longest_streak: number;
}

/** Phases the backend announces over the `state` event. */
export type Phase = 'idle' | 'recording' | 'processing';

interface TauriGlobal {
  core: { invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
  event: {
    listen: <T>(event: string, handler: (e: { payload: T }) => void) => Promise<() => void>;
  };
  window: { getCurrentWindow: () => AppWindow };
}

/**
 * The window controls the app draws for itself.
 *
 * The main window is created with `decorations: false`, so these are the only
 * way to minimise, maximise or close it from the webview. Each call is gated by
 * a `core:window:allow-*` permission in capabilities/default.json.
 */
export interface AppWindow {
  minimize: () => Promise<void>;
  toggleMaximize: () => Promise<void>;
  close: () => Promise<void>;
  isMaximized: () => Promise<boolean>;
  startDragging: () => Promise<void>;
}

function tauri(): TauriGlobal {
  const api = (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__;
  if (!api) {
    // Only reachable by opening the dev server in a plain browser, which the
    // app never does — the Rust side always injects the global.
    throw new Error('Not running inside Orra: the Tauri bridge is missing.');
  }
  return api;
}

export const appWindow = (): AppWindow => tauri().window.getCurrentWindow();

export const invoke = <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  tauri().core.invoke<T>(cmd, args);

export const listen = <T,>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> => tauri().event.listen<T>(event, (e) => handler(e.payload));

/* -------------------------------------------------------------- commands */

export const getStatus = () => invoke<Status>('get_status');
export const getHistory = () => invoke<Entry[]>('get_history');
export const getInsights = () => invoke<Insights>('get_insights');
export const listMics = () => invoke<string[]>('list_mics');
export const saveConfig = (config: Config) => invoke<string>('save_config', { config });
export const startDictation = () => invoke<void>('start_dictation');
export const stopDictation = () => invoke<void>('stop_dictation');
export const speak = (text: string) => invoke<void>('speak', { text });
export const stopSpeaking = () => invoke<void>('stop_speaking');
export const cycleLanguage = () => invoke<string>('cycle_language');
/** Switch to a specific dictation language and remember it. */
export const setLanguage = (code: string) => invoke<void>('set_language', { code });
export const verifyKey = () => invoke<string>('verify_key');
/** Checks the configured translation service by translating a short phrase. */
export const verifyTranslate = () => invoke<string>('verify_translate');
/**
 * The models an OpenAI-compatible endpoint has, for the fields that fill
 * themselves from it.
 *
 * The URL and key are passed rather than read from the saved settings, so the
 * button works on what is on screen before the debounced save has gone through.
 */
export const listModels = (base_url: string, api_key: string) =>
  invoke<string[]>('list_models', { baseUrl: base_url, apiKey: api_key });

/* ------------------------------------------------- running one on this machine */

/** One model Orra can fetch and run itself. */
export interface LocalModel {
  name: string;
  label: string;
  /** What it costs, in the words a user would use. */
  note: string;
  mb: number;
  installed: boolean;
}

/** Mirrors `engine::Availability`. */
export interface LocalAvailability {
  /** False where upstream publishes no engine build for this platform. */
  supported: boolean;
  /** True once the engine has been unpacked and is ready to start. */
  engine: boolean;
  running: boolean;
  /** The model the engine is running, if one is. */
  running_model: string | null;
  models: LocalModel[];
}

/** What a download is doing. Mirrors `engine::Progress`. */
export interface LocalProgress {
  /** `engine`, `starting`, or the model's name. */
  what: string;
  label: string;
  received: number;
  /** Zero when the size is not known — starting the engine reports no bytes. */
  total: number;
}

/** What running a local model looks like right now. */
export const localAvailability = () => invoke<LocalAvailability>('local_availability');

/**
 * Fetch a model — and the engine, once — and leave them on disk. Nothing runs
 * until {@link useLocalModel} says so.
 *
 * Resolves when the download has finished, which can be a long wait on a slow
 * connection; progress arrives meanwhile on the `local-download` event.
 */
export const downloadLocalModel = (name: string) => invoke<string>('download_local_model', { name });

/** Start the engine again on a model that is already downloaded. */
export const useLocalModel = (name: string) => invoke<string>('use_local_model', { name });

/** Stop the engine. The weights stay on disk. */
export const stopLocalEngine = () => invoke<void>('stop_local_engine');
export const applyHotkey = () => invoke<string>('apply_hotkey');
export const reinject = (id: string) => invoke<void>('reinject', { id });
export const deleteHistory = (id: string) => invoke<void>('delete_history', { id });
export const clearHistory = () => invoke<void>('clear_history');
export const quit = () => invoke<void>('quit');

/** Backend messages arrive as plain strings from `anyhow`. */
export function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

/**
 * The failure a rejected command carries, in the shape the card shows.
 *
 * The commands that talk to a service reject with a full `Problem`; the rest
 * still reject with a plain string, which is wrapped here with a headline that
 * claims nothing it cannot know. The text is its own log — there is no key in
 * "could not minimise the window" to take out.
 */
export function problemOf(e: unknown): Problem {
  if (e && typeof e === 'object' && 'log' in e && 'detail' in e) return e as Problem;
  const text = errorText(e);
  return {
    kind: 'unknown',
    title: 'Something went wrong',
    summary: text,
    advice: '',
    detail: text,
    log: text,
  };
}
