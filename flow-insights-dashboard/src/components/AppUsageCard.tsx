import {
  AppWindow,
  Code2,
  Globe,
  Mail,
  MessageSquare,
  Terminal,
  type LucideIcon,
} from 'lucide-react';

import { Card, Empty } from '@/components/ui';
import type { AppCount } from '@/lib/api';
import { num, prettifyApp } from '@/lib/stats';

/**
 * Which windows the words landed in, from the compositor's window class.
 *
 * The mockup listed invented categories ("6,594 AI PROMPTS") that nothing can
 * measure; this is the real thing, grouped by the class `hyprctl` reports for
 * whatever had focus.
 */
const HINTS: [RegExp, LucideIcon][] = [
  [/chrome|chromium|firefox|brave|zen|epiphany|vivaldi|opera/i, Globe],
  [/kitty|alacritty|ghostty|wezterm|konsole|terminal|foot|tmux|warp/i, Terminal],
  [/code|codium|zed|idea|jetbrains|sublime|emacs|neovim|nvim|android-studio/i, Code2],
  [/slack|discord|telegram|whatsapp|signal|element|teams|zoom/i, MessageSquare],
  [/mail|thunderbird|evolution|geary|outlook/i, Mail],
];

function iconFor(appClass: string): LucideIcon {
  for (const [pattern, Icon] of HINTS) {
    if (pattern.test(appClass)) return Icon;
  }
  return AppWindow;
}

export function AppUsageCard({ apps }: { apps: AppCount[] }) {
  // The bar lengths are relative to the busiest application, so the widest bar
  // is always full and the rest read against it.
  const busiest = apps.reduce((max, app) => Math.max(max, app.words), 0);
  // Shares are of the words that have a known target, not of every word ever
  // dictated, so the column adds up to the whole that is actually shown.
  const attributed = apps.reduce((n, app) => n + app.words, 0);

  return (
    <Card>
      <div className="flex items-center justify-between gap-3 mb-6">
        <h3 className="text-[22px] sm:text-[24px] font-bold tracking-tight">Where you dictate</h3>
        <span className="text-[11px] font-semibold text-muted tracking-[0.08em] uppercase">
          {num(apps.length)} {apps.length === 1 ? 'app' : 'apps'}
        </span>
      </div>

      {apps.length === 0 ? (
        <Empty>
          No target application recorded yet. The window class is read when a dictation finishes.
        </Empty>
      ) : (
        <div className="flex flex-col gap-4">
          {apps.map((app) => {
            const Icon = iconFor(app.app);
            const share = busiest > 0 ? app.words / busiest : 0;
            const percent = attributed > 0 ? Math.round((app.words / attributed) * 100) : 0;
            return (
              <div key={app.app} className="flex items-center gap-4">
                <div className="w-6 flex items-center justify-center shrink-0">
                  <Icon className="w-5 h-5" />
                </div>
                <div className="flex-1 flex items-center gap-3 min-w-0">
                  <div
                    className="h-7 bg-teal rounded-[6px] flex items-center px-2 shrink-0 transition-all duration-300"
                    // A floor of 8% so a small share is still a visible bar and
                    // still has room for its label.
                    style={{ width: `${Math.max(8, share * 100)}%` }}
                  >
                    <span className="text-[12px] font-bold text-white tracking-tight">
                      {percent}%
                    </span>
                  </div>
                  <span
                    className="text-[12px] font-bold tracking-wide whitespace-nowrap overflow-hidden text-ellipsis"
                    title={app.app}
                  >
                    {prettifyApp(app.app).toUpperCase()}
                  </span>
                </div>
                <span className="text-[12px] text-muted shrink-0">{num(app.words)}</span>
              </div>
            );
          })}
        </div>
      )}
    </Card>
  );
}
