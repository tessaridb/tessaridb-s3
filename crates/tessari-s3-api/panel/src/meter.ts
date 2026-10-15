//! How full a drive is: a bar on a zero baseline whose length is the used share,
//! with the figures beside it in text, so the bar's colour is never the only
//! signal. The thresholds turn it amber, then red, before the drive is full.

import { el } from "./dom.ts";
import { share, size } from "./format.ts";
import type { Drive } from "./models.ts";

/** Used share from which the bar warns, and from which it is critical. */
const WARN = 0.8;
const CRITICAL = 0.95;

/** Bytes in use: everything that is not free. */
export const used = (drive: Drive): number => Math.max(drive.capacity - drive.free, 0);

/** A meter for `drive`, labelled `label` for assistive technology. */
export function meter(drive: Drive, label: string): HTMLElement {
  const taken = used(drive);
  const ratio = drive.capacity === 0 ? 0 : taken / drive.capacity;
  const level = ratio >= CRITICAL ? "bad" : ratio >= WARN ? "warn" : "ok";
  const bar = el("div", {
    class: `meter ${level}`,
    role: "meter",
    "aria-label": label,
    "aria-valuemin": "0",
    "aria-valuemax": String(drive.capacity),
    "aria-valuenow": String(taken),
    "aria-valuetext": `${size(taken)} of ${size(drive.capacity)} used, ${share(taken, drive.capacity)}`,
  });
  bar.style.setProperty("--fill", ratio.toFixed(4));
  return el(
    "div",
    { class: "gauge" },
    bar,
    el("span", { class: "gauge-text" }, `${size(taken)} of ${size(drive.capacity)}`, el("span", { class: "muted" }, ` · ${share(taken, drive.capacity)}`)),
  );
}
