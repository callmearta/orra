import { useEffect, useRef, useState } from 'react';

/**
 * The rotating stamp in the corner.
 *
 * The mockup copied `window.location.href`, which in a desktop app is a
 * `tauri://localhost/…` string and means nothing to anyone. It now copies a
 * plain-text summary of the real figures, which is worth pasting somewhere.
 */
export function CircularShareBadge({ summary }: { summary: string }) {
  const [copied, setCopied] = useState(false);

  // Held in a ref and cleared on unmount: leaving the page within the 2.5s the
  // badge is showing would otherwise leave a timer that sets state on a
  // component React has already discarded.
  const copyTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(summary);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 2500);
    } catch {
      // A blocked clipboard is not worth an error banner; the button simply
      // does nothing.
    }
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={copy}
        className="relative w-14 h-14 rounded-full flex items-center justify-center hover:scale-105 active:scale-95 transition-transform group cursor-pointer"
        title="Copy a summary of these figures"
        aria-label="Copy a summary of these figures"
      >
        <svg
          className="absolute inset-0 w-full h-full animate-[spin_24s_linear_infinite] group-hover:animate-[spin_10s_linear_infinite] select-none pointer-events-none"
          viewBox="0 0 100 100"
        >
          <defs>
            <path
              id="stampCirclePath"
              d="M 50, 50 m -35, 0 a 35,35 0 1,1 70,0 a 35,35 0 1,1 -70,0"
            />
          </defs>
          <text
            className="text-[9.5px] font-bold fill-ink dark:fill-[#a1a1aa] tracking-[0.24em] uppercase"
            textLength="215"
            lengthAdjust="spacing"
          >
            <textPath href="#stampCirclePath" startOffset="0%">
              • COPY • COPY • COPY •
            </textPath>
          </text>
        </svg>

        <div className="relative z-10 text-ink group-hover:scale-110 transition-transform">
          <svg
            className="w-5 h-5"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <rect x="9" y="9" width="11" height="11" rx="2" />
            <path d="M5 15V5a2 2 0 0 1 2-2h10" />
          </svg>
        </div>
      </button>

      {copied && (
        <div className="absolute right-0 top-full mt-2 z-50 bg-ink text-canvas text-[11px] font-medium py-1.5 px-3 rounded-lg shadow-xl whitespace-nowrap">
          Summary copied
        </div>
      )}
    </div>
  );
}
