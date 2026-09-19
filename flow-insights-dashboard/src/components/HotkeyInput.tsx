/**
 * A text box that captures the next key combination pressed into it.
 *
 * The spelling it produces (`SUPER + ALT + D`) is the one already saved in
 * `~/.config/orra/config.json` and written verbatim into `keybinds.lua`, so
 * it is not free to change.
 */

import { useState } from 'react';

import { Input } from '@/components/ui';

const MODIFIER_LABEL: Record<string, string> = {
  Control: 'CTRL',
  Shift: 'SHIFT',
  Alt: 'ALT',
  Meta: 'SUPER',
};

function keyLabel(code: string, key: string): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  switch (code) {
    case 'Space':
      return 'SPACE';
    case 'Enter':
    case 'NumpadEnter':
      return 'RETURN';
    case 'Escape':
      return 'ESCAPE';
    case 'Tab':
      return 'TAB';
    case 'Backspace':
      return 'BACKSPACE';
    default:
      break;
  }
  if (code.startsWith('Arrow')) return code.slice(5).toUpperCase();
  return (key || code).toUpperCase();
}

export function HotkeyInput({
  value,
  onChange,
  label,
}: {
  value: string;
  onChange: (hotkey: string) => void;
  label: string;
}) {
  const [listening, setListening] = useState(false);

  return (
    <Input
      readOnly
      aria-label={label}
      value={listening ? 'Press your keys…' : value}
      placeholder="Click, then press your keys"
      className={listening ? 'border-teal cursor-crosshair' : 'cursor-pointer'}
      onClick={() => setListening(true)}
      onBlur={() => setListening(false)}
      onKeyDown={(event) => {
        if (!listening) return;
        // Everything is swallowed while capturing, so the keys cannot also
        // drive the page.
        event.preventDefault();

        if (event.key === 'Escape') {
          setListening(false);
          return;
        }
        // A bare modifier is not a shortcut yet — wait for the real key.
        if (MODIFIER_LABEL[event.key]) return;

        const parts: string[] = [];
        if (event.ctrlKey) parts.push('CTRL');
        if (event.altKey) parts.push('ALT');
        if (event.shiftKey) parts.push('SHIFT');
        if (event.metaKey) parts.push('SUPER');
        parts.push(keyLabel(event.code, event.key));

        onChange(parts.join(' + '));
        setListening(false);
      }}
    />
  );
}
