/**
 * The app's shared state: config, history, insights, the live dictation and the
 * event subscriptions that keep them current.
 *
 * Every control saves itself — there is no Save button — so `update` merges a
 * patch, renders immediately and debounces the write to disk.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';

import * as api from './lib/api';
import type { Config, Entry, Insights, Phase, Status } from './lib/api';

const SAVE_DEBOUNCE_MS = 600;

export interface Banner {
  message: string;
  kind: 'info' | 'ok' | 'err';
}

interface Live {
  phase: Phase;
  final: string;
  interim: string;
  level: number;
}

interface Store {
  ready: boolean;
  bootError: string | null;
  status: Status | null;
  config: Config | null;
  history: Entry[];
  insights: Insights | null;
  mics: string[];
  live: Live;
  banner: Banner | null;
  notify: (message: string, kind?: Banner['kind']) => void;
  /** Merge a change and save it once the user stops fiddling. */
  update: (patch: Partial<Config>) => void;
  /** Merge a change and save at once, for things that must apply now. */
  updateNow: (patch: Partial<Config>) => Promise<void>;
  refreshStatus: () => Promise<void>;
  refreshHistory: () => Promise<void>;
}

const StoreContext = createContext<Store | null>(null);

/**
 * Backend notes that mean the change did not fully apply.
 *
 * Matched on whole words: without the boundaries, "not" fires inside "note" and
 * "nothing", so an ordinary status line would surface as a red error banner.
 */
const TROUBLE = /\b(error|not|cannot|could not)\b/i;

export function StoreProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [history, setHistory] = useState<Entry[]>([]);
  const [insights, setInsights] = useState<Insights | null>(null);
  const [mics, setMics] = useState<string[]>([]);
  const [live, setLive] = useState<Live>({ phase: 'idle', final: '', interim: '', level: 0 });
  const [banner, setBanner] = useState<Banner | null>(null);

  const notify = useCallback((message: string, kind: Banner['kind'] = 'info') => {
    setBanner({ message, kind });
  }, []);

  // The banner clears itself; the timer is held in a ref so a second message
  // replaces the first cleanly instead of both timers racing.
  const bannerTimer = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (!banner) return;
    window.clearTimeout(bannerTimer.current);
    bannerTimer.current = window.setTimeout(
      () => setBanner(null),
      banner.kind === 'err' ? 8000 : 4000,
    );
    return () => window.clearTimeout(bannerTimer.current);
  }, [banner]);

  const refreshStatus = useCallback(async () => {
    const next = await api.getStatus();
    setStatus(next);
    setConfig(next.config);
  }, []);

  const refreshHistory = useCallback(async () => {
    const [entries, next] = await Promise.all([api.getHistory(), api.getInsights()]);
    setHistory(entries);
    setInsights(next);
  }, []);

  /* ------------------------------------------------------------- saving */

  const saveTimer = useRef<number | undefined>(undefined);
  /**
   * The newest config, so a debounced write persists what the user last set
   * rather than whatever its closure captured. Kept in step with the state and
   * advanced on every merge, so two changes in one tick compose.
   */
  const latest = useRef<Config | null>(null);
  useEffect(() => {
    latest.current = config;
  }, [config]);

  const write = useCallback(
    async (next: Config) => {
      try {
        const note = await api.saveConfig(next);
        if (note && TROUBLE.test(note)) notify(note, 'err');
      } catch (e) {
        notify(api.errorText(e), 'err');
      }
    },
    [notify],
  );

  const update = useCallback(
    (patch: Partial<Config>) => {
      const current = latest.current;
      if (!current) return;
      const next = { ...current, ...patch };
      latest.current = next;
      setConfig(next);
      // The debounce is scheduled out here rather than inside the setState
      // updater: an updater has to be pure, and React double-invokes it under
      // StrictMode, which would arm two timers and drop one of the writes.
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => void write(next), SAVE_DEBOUNCE_MS);
    },
    [write],
  );

  const updateNow = useCallback(
    async (patch: Partial<Config>) => {
      const current = latest.current;
      if (!current) return;
      window.clearTimeout(saveTimer.current);
      const next = { ...current, ...patch };
      latest.current = next;
      setConfig(next);
      await write(next);
    },
    [write],
  );

  /* --------------------------------------------------------------- boot */

  useEffect(() => {
    let unlisten: (() => void)[] = [];
    let cancelled = false;

    (async () => {
      try {
        await refreshStatus();
        if (cancelled) return;
        // Device enumeration and history are best-effort: neither should stop
        // the window from opening.
        await Promise.all([
          refreshHistory(),
          api.listMics().then(setMics).catch(() => setMics([])),
        ]);
        if (cancelled) return;

        unlisten = await Promise.all([
          api.listen<{ final: string; interim: string }>('stt', (payload) =>
            setLive((l) => ({ ...l, final: payload.final ?? '', interim: payload.interim ?? '' })),
          ),
          api.listen<number>('mic-level', (level) =>
            setLive((l) => ({ ...l, level: Math.max(0, Math.min(1, level || 0)) })),
          ),
          api.listen<Phase>('state', (phase) => {
            const next: Phase =
              phase === 'recording' || phase === 'processing' ? phase : 'idle';
            setLive((l) =>
              next === 'recording'
                ? // A fresh dictation starts from a clean slate.
                  { phase: next, final: '', interim: '', level: 0 }
                : { ...l, phase: next, level: next === 'idle' ? 0 : l.level },
            );
            // The slot has been vacated and the history file appended to, so
            // this is the moment the figures change.
            if (next === 'idle') void refreshHistory();
          }),
          api.listen<{ code: string }>('language', (payload) => {
            if (!payload?.code) return;
            setConfig((c) => (c ? { ...c, language: payload.code } : c));
          }),
          api.listen<void>('history', () => void refreshHistory()),
          api.listen<string>('error', (message) => notify(message, 'err')),
          api.listen<void>('status', () => void refreshStatus()),
        ]);
        if (cancelled) {
          unlisten.forEach((off) => off());
          return;
        }
        setReady(true);
      } catch (e) {
        if (!cancelled) setBootError(api.errorText(e));
      }
    })();

    return () => {
      cancelled = true;
      unlisten.forEach((off) => off());
    };
  }, [notify, refreshHistory, refreshStatus]);

  const value = useMemo<Store>(
    () => ({
      ready,
      bootError,
      status,
      config,
      history,
      insights,
      mics,
      live,
      banner,
      notify,
      update,
      updateNow,
      refreshStatus,
      refreshHistory,
    }),
    [
      ready,
      bootError,
      status,
      config,
      history,
      insights,
      mics,
      live,
      banner,
      notify,
      update,
      updateNow,
      refreshStatus,
      refreshHistory,
    ],
  );

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}

export function useStore(): Store {
  const store = useContext(StoreContext);
  if (!store) throw new Error('useStore must be used inside StoreProvider');
  return store;
}
