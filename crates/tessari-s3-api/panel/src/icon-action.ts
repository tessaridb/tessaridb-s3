//! A row's action as an icon button with a tooltip. The accessible name says
//! the action and the row it acts on ("Revoke key TS…"); the tooltip says the
//! action alone, because the row is beside it. The tooltip is one shared
//! element placed over the page, so a scrolling table never clips it; it opens
//! after a short pause under the pointer — at once when another tip was just
//! open, or when the button takes keyboard focus — and Escape closes it.

import { el } from "./dom.ts";
import { icon, type IconName } from "./icons.ts";

/** Pointer pause before the first tip opens, and how long an open tip makes the next one instant. */
const OPEN_DELAY_MS = 500;
const INSTANT_WINDOW_MS = 400;

export function iconAction(name: IconName, tip: string, label: string, tone: "quiet" | "danger" = "quiet"): HTMLButtonElement {
  return el("button", { type: "button", class: `icon-action ${tone}`, "aria-label": label, "data-tip": tip }, icon(name));
}

/** Wires the shared tooltip to every `[data-tip]` control, now and later; called once. */
export function installTips(): void {
  const tip = el("div", { class: "tip", role: "tooltip", "aria-hidden": "true" });
  document.body.append(tip);
  let anchor: HTMLElement | null = null;
  let timer: number | undefined;
  let closedAt = Number.NEGATIVE_INFINITY;
  const place = (target: HTMLElement): void => {
    const box = target.getBoundingClientRect();
    const own = tip.getBoundingClientRect();
    const above = box.top - own.height - 6;
    const below = above < 4;
    const left = Math.min(Math.max(4, box.left + box.width / 2 - own.width / 2), window.innerWidth - own.width - 4);
    tip.style.left = `${left}px`;
    tip.style.top = `${below ? box.bottom + 6 : above}px`;
    tip.dataset["side"] = below ? "below" : "above";
  };
  const show = (target: HTMLElement, instant: boolean): void => {
    anchor = target;
    tip.textContent = target.dataset["tip"] ?? "";
    tip.toggleAttribute("data-instant", instant);
    tip.dataset["open"] = "";
    place(target);
  };
  const hide = (): void => {
    window.clearTimeout(timer);
    if (anchor !== null) {
      closedAt = performance.now();
    }
    anchor = null;
    delete tip.dataset["open"];
  };
  const target = (event: Event): HTMLElement | null =>
    event.target instanceof Element ? event.target.closest<HTMLElement>("[data-tip]") : null;
  document.addEventListener("pointerover", (event) => {
    const over = target(event);
    if (over === null || over === anchor) {
      return;
    }
    hide();
    const instant = performance.now() - closedAt < INSTANT_WINDOW_MS;
    timer = window.setTimeout(() => show(over, instant), instant ? 0 : OPEN_DELAY_MS);
  });
  document.addEventListener("pointerout", (event) => {
    if (target(event) !== null) {
      hide();
    }
  });
  document.addEventListener("focusin", (event) => {
    const focused = target(event);
    if (focused !== null && focused.matches(":focus-visible")) {
      hide();
      show(focused, true);
    }
  });
  document.addEventListener("focusout", hide);
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      hide();
    }
  });
  window.addEventListener("scroll", hide, { capture: true, passive: true });
}
