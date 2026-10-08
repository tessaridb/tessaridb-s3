//! How figures are written: sizes in binary units, times in the reader's locale.

const UNITS = ["KiB", "MiB", "GiB", "TiB", "PiB"] as const;
const whole = new Intl.NumberFormat("en", { maximumFractionDigits: 0 });
const tenth = new Intl.NumberFormat("en", { minimumFractionDigits: 1, maximumFractionDigits: 1 });

/** A byte count for a reader: exact below a KiB, one decimal in the largest unit at or above it. */
export function size(bytes: number): string {
  if (bytes < 1024) {
    return `${whole.format(bytes)} B`;
  }
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${tenth.format(value)} ${UNITS[unit] ?? "PiB"}`;
}

const stamp = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "medium" });

/** An ISO-8601 instant in the reader's locale and zone; the text itself when it is not one. */
export function moment(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? iso : stamp.format(at);
}

const percent = new Intl.NumberFormat("en", { style: "percent", maximumFractionDigits: 0 });

/** The share `part` is of `whole`, as a percentage; 0 % of nothing. */
export const share = (part: number, whole: number): string => percent.format(whole === 0 ? 0 : part / whole);

/** A count for a reader, grouped by the reader's locale. */
export const amount = (value: number): string => value.toLocaleString();
