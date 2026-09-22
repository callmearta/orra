/**
 * Small, UI-facing validators.
 *
 * These are intentionally conservative: the backend still owns the real
 * provider errors, but the form should not let an obviously malformed value
 * look accepted while a request is in flight.
 */

export const blank = (value: string | null | undefined): boolean => !value || value.trim() === '';

export function required(value: string | null | undefined, label = 'This field'): string | null {
  return blank(value) ? `${label} is required.` : null;
}

export function httpUrl(value: string | null | undefined, label = 'URL'): string | null {
  if (blank(value)) return `${label} is required.`;
  const raw = value!.trim();
  if (/\s/.test(raw)) return `${label} cannot contain spaces.`;
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return `${label} must start with http:// or https://.`;
    }
    return null;
  } catch {
    return `Enter a valid ${label.toLowerCase()} like http://localhost:8080/v1.`;
  }
}

export function optionalHttpUrl(value: string | null | undefined, label = 'URL'): string | null {
  return blank(value) ? null : httpUrl(value, label);
}

export function duplicate(value: string, values: string[], label = 'This value'): string | null {
  if (blank(value)) return null;
  const needle = value.trim().toLowerCase();
  return values.some((item) => item !== value && item.trim().toLowerCase() === needle)
    ? `${label} is already in the list.`
    : null;
}

export function languageCycleError(codes: string[]): string | null {
  if (codes.some((code) => blank(code))) return 'Remove empty languages or choose a language for each row.';
  const seen = new Set<string>();
  for (const code of codes) {
    const key = code.trim().toLowerCase();
    if (seen.has(key)) return 'Each language can appear only once in the switch list.';
    seen.add(key);
  }
  return null;
}
