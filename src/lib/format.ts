// Small text helpers. Hand written: the launcher shows two kinds of numbers,
// which does not justify a formatting library.

const MEGABYTE = 1024 * 1024;

/** A byte count as megabytes with one decimal: `18.0 MB`. */
export function megabytes(bytes: number): string {
  return `${(bytes / MEGABYTE).toFixed(1)} MB`;
}

/** Progress as `11.2 of 18.0 MB`. */
export function megabytesOf(received: number, total: number): string {
  return `${(received / MEGABYTE).toFixed(1)} of ${megabytes(total)}`;
}

/** A byte count in the unit that fits it: `512 B`, `3.4 KB`, `18.0 MB`. */
export function bytes(count: number): string {
  if (count < 1024) return `${count} B`;
  if (count < MEGABYTE) return `${(count / 1024).toFixed(1)} KB`;
  return megabytes(count);
}

/** A whole percentage from 0 to 100, also when the total is unknown or zero. */
export function percent(received: number, total: number): number {
  if (!(total > 0) || !Number.isFinite(received)) return 0;
  return Math.min(100, Math.max(0, Math.round((received / total) * 100)));
}

/** Text from the Rust side as a sentence: it arrives without a final full stop. */
export function sentence(text: string): string {
  return /[.!?]$/.test(text) ? text : `${text}.`;
}
