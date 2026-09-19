/**
 * The strip the app draws across the top of itself.
 *
 * The main window is created with `decorations: false`, so this *is* the title
 * bar: the lights on the left really minimise, maximise and close, and the strip
 * behind them is a drag region. Closing hides the window and leaves dictation
 * running in the tray, which is what the native close button used to do.
 */

import { Moon, PanelLeft, Sun } from 'lucide-react';

import * as api from '@/lib/api';
import { useStore } from '@/store';

const LIGHTS = [
  {
    key: 'close',
    // The coloured dot is the visual; the glyph appears on hover, as it does in
    // the systems this is imitating.
    className: 'bg-[#ff5f57] border-[#e0443e]',
    glyph: '✕',
    title: 'Close — keeps dictating in the background',
    run: (w: api.AppWindow) => w.close(),
  },
  {
    key: 'minimize',
    className: 'bg-[#febc2e] border-[#d89e24]',
    glyph: '–',
    title: 'Minimise',
    run: (w: api.AppWindow) => w.minimize(),
  },
  {
    key: 'maximize',
    className: 'bg-[#28c840] border-[#1aab29]',
    glyph: '+',
    title: 'Maximise',
    run: (w: api.AppWindow) => w.toggleMaximize(),
  },
] as const;

export function WindowChrome({
  darkMode,
  onToggleDarkMode,
  sidebarOpen,
  onToggleSidebar,
  version,
}: {
  darkMode: boolean;
  onToggleDarkMode: () => void;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  version?: string;
}) {
  const { notify } = useStore();

  const run = async (light: (typeof LIGHTS)[number]) => {
    try {
      await light.run(api.appWindow());
    } catch (e) {
      // A refused window call means a missing permission, which is worth
      // saying out loud rather than leaving a dead button.
      notify(`Could not ${light.key} the window: ${api.errorText(e)}`, 'err');
    }
  };

  return (
    <header
      // Lets the compositor move the window by this strip. Buttons inside are
      // their own targets, so they still receive clicks.
      data-tauri-drag-region
      className="h-12 px-5 flex items-center justify-between select-none shrink-0"
    >
      <div className="flex items-center gap-5">
        <div className="flex items-center gap-2 group">
          {LIGHTS.map((light) => (
            <button
              key={light.key}
              type="button"
              onClick={() => void run(light)}
              title={light.title}
              aria-label={light.title}
              className={`w-3 h-3 rounded-full border cursor-pointer inline-flex items-center justify-center text-[8px] leading-none text-black/60 opacity-90 hover:opacity-100 ${light.className}`}
            >
              <span className="opacity-0 group-hover:opacity-100 transition-opacity">
                {light.glyph}
              </span>
            </button>
          ))}
        </div>

        <button
          type="button"
          onClick={onToggleSidebar}
          className={`p-1 rounded-md text-muted hover:text-ink transition-colors cursor-pointer ${
            !sidebarOpen ? 'bg-black/10 dark:bg-white/10' : ''
          }`}
          title={sidebarOpen ? 'Hide sidebar' : 'Show sidebar'}
          aria-label={sidebarOpen ? 'Hide sidebar' : 'Show sidebar'}
        >
          <PanelLeft className="w-4 h-4" />
        </button>
      </div>

      <div className="flex items-center gap-4 text-muted">
        {version && <span className="text-[11px] font-semibold tracking-wide">v{version}</span>}
        <button
          type="button"
          onClick={onToggleDarkMode}
          className="p-1 hover:text-ink transition-colors cursor-pointer"
          title={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
          aria-label={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
        >
          {darkMode ? <Sun className="w-[18px] h-[18px]" /> : <Moon className="w-[18px] h-[18px]" />}
        </button>
      </div>
    </header>
  );
}
