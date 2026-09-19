/**
 * The icon rail.
 *
 * Every entry leads to a page that works. The mockup listed fourteen, but nine
 * of them (AI Enhancer, Voice Chat, Action Items, Developer API, Community,
 * Refer & Earn, Help & Docs, Applications, Audio Engine) had no backend behind
 * them at all, so they are gone rather than present and dead.
 */

import type { ComponentType } from 'react';
import {
  AudioLines,
  BarChart2,
  CircleDot,
  FileText,
  Scissors,
  Settings,
} from 'lucide-react';

export type NavId =
  | 'dictate'
  | 'insights'
  | 'transcripts'
  | 'snippets'
  | 'dictionary'
  | 'voice'
  | 'settings';

interface NavItem {
  id: NavId;
  label: string;
  icon?: ComponentType<{ className?: string }>;
  customIcon?: ComponentType<{ className?: string }>;
}

/** The "Tt" glyph from the mockup, kept because lucide has no dictionary mark. */
function TtIcon({ className = 'w-5 h-5' }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M4 6V4h10v2" />
      <path d="M9 4v14" />
      <path d="M15 10v-1h6v1" />
      <path d="M18 9v9" />
    </svg>
  );
}

/** The four-bar mark the app is known by. */
function WaveformLogo({ className = '' }: { className?: string }) {
  return (
    <div className={`t-wave flex items-center gap-[3px] h-7 ${className}`}>
      <span className="w-[3px] h-[12px] bg-current rounded-full" />
      <span className="w-[3px] h-[24px] bg-current rounded-full" />
      <span className="w-[3px] h-[18px] bg-current rounded-full" />
      <span className="w-[3px] h-[13px] bg-current rounded-full" />
    </div>
  );
}

const MAIN: NavItem[] = [
  { id: 'dictate', label: 'Dictate', icon: CircleDot },
  { id: 'insights', label: 'Insights', icon: BarChart2 },
  { id: 'transcripts', label: 'Transcripts & Notes', icon: FileText },
  { id: 'snippets', label: 'Snippets & Shortcuts', icon: Scissors },
  { id: 'dictionary', label: 'Dictionary & Formatting', customIcon: TtIcon },
  { id: 'voice', label: 'Voice', icon: AudioLines },
];

const BOTTOM: NavItem[] = [{ id: 'settings', label: 'Settings', icon: Settings }];

function Item({ item, active, onSelect }: { item: NavItem; active: boolean; onSelect: (id: NavId) => void }) {
  const Icon = item.icon;
  const Custom = item.customIcon;
  return (
    <button
      type="button"
      onClick={() => onSelect(item.id)}
      title={item.label}
      aria-label={item.label}
      aria-current={active ? 'page' : undefined}
      className={`w-9 h-9 flex items-center justify-center rounded-[8px] transition-all duration-150 cursor-pointer ${
        active
          ? 'border-[1.5px] border-ink text-ink bg-black/5 dark:bg-white/10'
          : 'text-muted hover:text-ink border-[1.5px] border-transparent'
      }`}
    >
      {Icon ? <Icon className="w-[19px] h-[19px]" /> : Custom ? <Custom className="w-[19px] h-[19px]" /> : null}
    </button>
  );
}

export function Sidebar({
  activeNav,
  onNavSelect,
}: {
  activeNav: NavId;
  onNavSelect: (id: NavId) => void;
}) {
  return (
    <aside className="w-14 sm:w-16 flex flex-col items-center justify-between py-5 select-none shrink-0">
      <div className="flex flex-col items-center gap-6 w-full">
        <button
          type="button"
          onClick={() => onNavSelect('dictate')}
          className="text-ink hover:opacity-75 transition-opacity cursor-pointer mb-2"
          aria-label="Orra home"
        >
          <WaveformLogo />
        </button>

        <nav className="flex flex-col items-center gap-3 w-full">
          {MAIN.map((item) => (
            <Item key={item.id} item={item} active={activeNav === item.id} onSelect={onNavSelect} />
          ))}
        </nav>
      </div>

      <div className="flex flex-col items-center gap-3 w-full pt-4">
        {BOTTOM.map((item) => (
          <Item key={item.id} item={item} active={activeNav === item.id} onSelect={onNavSelect} />
        ))}
      </div>
    </aside>
  );
}
