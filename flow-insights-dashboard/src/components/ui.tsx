/**
 * Small shared pieces so the pages stay readable.
 *
 * These use the semantic tokens from index.css rather than the arbitrary hex
 * values the original cards were written with, which means they re-theme for
 * dark mode on their own.
 */

import { useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes } from 'react';
import {
  Check,
  Copy,
  KeyRound,
  MonitorX,
  SearchX,
  Settings,
  Timer,
  TriangleAlert,
  WifiOff,
  X,
} from 'lucide-react';

import type { Problem, ProblemKind } from '@/lib/api';
import { cn } from '@/lib/utils';

/* --------------------------------------------------------------- layout */

export function Card({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('bg-card rounded-[22px] p-6 sm:p-7 transition-colors', className)}>
      {children}
    </div>
  );
}

export function PageHeading({
  title,
  lede,
  aside,
}: {
  title: string;
  // A lede can carry inline emphasis, so it is not restricted to a plain string.
  lede?: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div>
        <h1 className="text-[26px] sm:text-[28px] font-bold tracking-tight">{title}</h1>
        {lede && <p className="text-[14px] text-muted mt-1 max-w-2xl">{lede}</p>}
      </div>
      {aside && <div className="shrink-0">{aside}</div>}
    </div>
  );
}

export function SectionTitle({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 mb-3 mt-7 first:mt-0">
      <h2 className="text-[18px] font-bold tracking-tight">{children}</h2>
      {aside && (
        <span className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted">
          {aside}
        </span>
      )}
    </div>
  );
}

/** A labelled block inside a card. */
export function Row({
  label,
  sub,
  children,
  className,
}: {
  label: ReactNode;
  sub?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn('flex items-center justify-between gap-5', className)}>
      <div className="min-w-0">
        <div className="text-[14px] font-semibold">{label}</div>
        {sub && <div className="text-[12px] text-muted mt-0.5">{sub}</div>}
      </div>
      {children && <div className="shrink-0">{children}</div>}
    </div>
  );
}

/** Divider between rows, matching the card hairlines. */
export function Divider() {
  return <div className="border-t border-hair my-4" />;
}

export function Label({ children, htmlFor }: { children: ReactNode; htmlFor?: string }) {
  return (
    <label htmlFor={htmlFor} className="block text-[12px] font-semibold text-muted mb-1.5">
      {children}
    </label>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="text-[13px] text-muted py-6 text-center">{children}</div>;
}

export function Pill({
  children,
  tone = 'neutral',
  className,
}: {
  children: ReactNode;
  tone?: 'neutral' | 'good' | 'bad' | 'teal';
  className?: string;
}) {
  const tones = {
    neutral: 'bg-black/5 text-muted dark:bg-white/10',
    good: 'bg-good text-good-ink',
    bad: 'bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-200',
    teal: 'bg-teal text-white',
  };
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 text-[11px] font-bold px-2.5 py-1 rounded-full',
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

/* ------------------------------------------------------------- controls */

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'ghost' | 'danger' | 'mini';
};

export function Button({ variant = 'ghost', className, ...rest }: ButtonProps) {
  const variants = {
    primary: 'bg-teal text-white hover:brightness-110 px-4 py-2.5 text-[13px]',
    ghost: 'border border-hair hover:bg-black/5 dark:hover:bg-white/10 px-4 py-2.5 text-[13px]',
    danger:
      'border border-red-300 text-red-700 hover:bg-red-50 dark:border-red-900 dark:text-red-300 dark:hover:bg-red-950 px-4 py-2.5 text-[13px]',
    mini: 'border border-hair hover:bg-black/5 dark:hover:bg-white/10 px-2.5 py-1 text-[12px]',
  };
  return (
    <button
      type="button"
      className={cn(
        'inline-flex items-center justify-center gap-1.5 rounded-xl font-semibold transition-colors',
        'disabled:opacity-40 disabled:pointer-events-none cursor-pointer',
        variants[variant],
        className,
      )}
      {...rest}
    />
  );
}

export function Toggle({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative w-[42px] h-[24px] rounded-full transition-colors cursor-pointer shrink-0',
        checked ? 'bg-teal' : 'bg-black/15 dark:bg-white/20',
      )}
    >
      <span
        className={cn(
          'absolute top-[3px] w-[18px] h-[18px] rounded-full bg-white shadow-sm transition-all',
          checked ? 'left-[21px]' : 'left-[3px]',
        )}
      />
    </button>
  );
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        'w-full bg-sheet border border-hair rounded-xl px-3 py-2 text-[14px]',
        // The border tint alone is too faint to serve as a focus indicator, so
        // keyboard focus also gets a ring; `outline-none` only takes away the
        // native ring, which is replaced by the two above.
        'placeholder:text-muted/60 focus:outline-none focus:border-teal focus-visible:ring-2 focus-visible:ring-teal/40',
        className,
      )}
      {...rest}
    />
  );
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      className={cn(
        'w-full bg-sheet border border-hair rounded-xl px-3 py-2 text-[14px]',
        'focus:outline-none focus:border-teal focus-visible:ring-2 focus-visible:ring-teal/40 cursor-pointer',
        className,
      )}
      {...rest}
    >
      {children}
    </select>
  );
}

/* ---------------------------------------------------------------- misc */

/** The floating info/confirmation strip at the top of the sheet. */
export function Banner({ message, kind }: { message: string; kind: 'info' | 'ok' }) {
  const tones = {
    info: 'bg-black/5 text-ink dark:bg-white/10',
    ok: 'bg-good text-good-ink',
  };
  return (
    <div className={cn('rounded-xl px-4 py-2.5 text-[13px] font-medium mb-4', tones[kind])}>
      {message}
    </div>
  );
}

/**
 * What each kind of failure is marked with, so the headline is readable before
 * it is read: a connection problem, a key, a setting nobody filled in.
 */
const PROBLEM_ICON: Record<ProblemKind, typeof TriangleAlert> = {
  network: WifiOff,
  auth: KeyRound,
  quota: Timer,
  request: SearchX,
  config: Settings,
  local: MonitorX,
  unknown: TriangleAlert,
};

/**
 * A failure, at the top of the sheet.
 *
 * Deliberately not the info strip with another colour. It does not time out —
 * the failures that matter here happen while the user is looking at another
 * window, so it waits to be read — and it carries three things the strip has no
 * room for: what kind of failure it was, the error itself, and one button that
 * puts the whole thing, version and platform included, on the clipboard for a
 * bug report.
 */
export function ProblemCard({ problem, onClose }: { problem: Problem; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const Icon = PROBLEM_ICON[problem.kind] ?? TriangleAlert;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(problem.log);
      setCopied(true);
    } catch {
      // The webview can refuse the clipboard. The log is on screen either way,
      // so there is nothing to report here — and this is the one component
      // that must not answer a failure with a failure.
    }
  };

  return (
    <div className="rounded-xl bg-red-100 dark:bg-red-950 text-red-900 dark:text-red-100 px-4 py-3 mb-4">
      <div className="flex items-start gap-3">
        <Icon className="w-4 h-4 mt-[3px] shrink-0" aria-hidden />
        <div className="flex-1 min-w-0">
          <p className="text-[13px] font-semibold">
            {problem.title}
            {problem.summary && (
              <span className="font-normal opacity-90"> — {problem.summary}</span>
            )}
          </p>
          {problem.advice && <p className="text-[12px] mt-1 opacity-90">{problem.advice}</p>}
          {/* The raw error, wrapped and selectable, because reading it is the
              point of showing it at all. */}
          <p className="text-[11px] mt-2 font-mono break-words whitespace-pre-wrap opacity-75">
            {problem.detail}
          </p>
          <button
            type="button"
            onClick={() => void copy()}
            className={cn(
              'inline-flex items-center gap-1.5 mt-3 px-2.5 py-1 rounded-lg text-[12px] font-semibold cursor-pointer',
              'border border-red-300/70 hover:bg-red-200/50 dark:border-red-900 dark:hover:bg-red-900/50',
            )}
          >
            {copied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
            {copied ? 'Copied' : 'Copy log'}
          </button>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Dismiss"
          className="shrink-0 p-1 rounded-lg cursor-pointer opacity-70 hover:opacity-100 hover:bg-red-200/50 dark:hover:bg-red-900/50"
        >
          <X className="w-4 h-4" />
        </button>
      </div>
    </div>
  );
}

/** The little live mic meter used on the Dictate page. */
export function Meter({ level, active }: { level: number; active: boolean }) {
  const bars = 15;
  return (
    <div className={cn('flex items-end justify-center gap-[3px] h-[46px]', !active && 'opacity-40')}>
      {Array.from({ length: bars }, (_, i) => {
        // A gentle bell across the bars keeps it reading as a waveform rather
        // than a bar chart.
        const shape = 0.45 + 0.55 * Math.sin((Math.PI * (i + 0.5)) / bars);
        const height = active ? 6 + Math.min(1, level * 3.2) * 34 * shape : 6;
        return (
          <span
            key={i}
            className="w-[4px] rounded-full bg-teal-soft transition-all duration-75"
            style={{ height: `${height.toFixed(1)}px` }}
          />
        );
      })}
    </div>
  );
}
