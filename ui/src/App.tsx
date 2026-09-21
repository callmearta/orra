import { useEffect, useState } from 'react';

import { Sidebar, type NavId } from '@/components/Sidebar';
import { Banner, ProblemCard } from '@/components/ui';
import { WindowChrome } from '@/components/WindowChrome';

import DictionaryPage from './pages/Dictionary';
import DictatePage from './pages/Dictate';
import InsightsPage from './pages/Insights';
import SettingsPage from './pages/Settings';
import SnippetsPage from './pages/Snippets';
import TranscriptsPage from './pages/Transcripts';
import VoicePage from './pages/Voice';
import { StoreProvider, useStore } from './store';

function Placeholder({ message }: { message: string }) {
  return <div className="text-[14px] text-muted py-20 text-center">{message}</div>;
}

function Shell() {
  const { ready, bootError, config, status, banner, problem, dismissProblem, update } = useStore();
  const [nav, setNav] = useState<NavId>('dictate');
  const [sidebarOpen, setSidebarOpen] = useState(true);

  const dark = config?.dark_mode ?? false;

  // The token overrides in index.css re-point --color-ink and friends on `.dark`.
  // That class has to sit on <html>, not on the shell below: `body { color:
  // var(--color-ink) }` is inherited from outside the shell, so a `.dark` div
  // would leave every element without an explicit text utility on the light
  // palette's near-black ink, rendering it illegible on the dark background.
  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark);
    document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  }, [dark]);

  const page = (() => {
    switch (nav) {
      case 'dictate':
        return <DictatePage onOpenSettings={() => setNav('settings')} />;
      case 'insights':
        return <InsightsPage />;
      case 'transcripts':
        return <TranscriptsPage />;
      case 'snippets':
        return <SnippetsPage />;
      case 'dictionary':
        return <DictionaryPage />;
      case 'voice':
        return <VoicePage />;
      case 'settings':
        return <SettingsPage />;
    }
  })();

  return (
    // The app *is* the window here. The mockup drew a fake desktop with a fake
    // macOS window sitting on it, which in a real, natively-decorated window
    // just reads as a window inside a window — so that outer frame is gone and
    // only the shell below it is kept.
    <div className="h-screen w-full flex flex-col bg-canvas font-sans">
      <WindowChrome
        darkMode={dark}
        onToggleDarkMode={() => update({ dark_mode: !dark })}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((open) => !open)}
        version={status?.version}
      />

      {/* min-h-0 lets the sheet shrink so that it, not the window, scrolls. */}
      <div className="flex flex-1 min-h-0 p-2 sm:p-3 sm:pt-0 gap-2 sm:gap-3">
        {sidebarOpen && <Sidebar activeNav={nav} onNavSelect={setNav} />}

        <main className="flex-1 min-w-0 rounded-[20px] sm:rounded-[28px] bg-sheet p-6 sm:p-8 md:p-10 overflow-y-auto">
          {/* One strip at a time: a failure already on screen is the more
              important of the two, and it stays until it is dismissed. */}
          {problem ? (
            <ProblemCard problem={problem} onClose={dismissProblem} />
          ) : (
            banner && <Banner message={banner.message} kind={banner.kind} />
          )}
          {bootError ? (
            <Placeholder message={`Could not start the interface: ${bootError}`} />
          ) : !ready || !config ? (
            <Placeholder message="Starting…" />
          ) : (
            page
          )}
        </main>
      </div>
    </div>
  );
}

export default function App() {
  return (
    <StoreProvider>
      <Shell />
    </StoreProvider>
  );
}
