//! What every view is handed: the region it draws in, a way to know it is still
//! the view on screen (a slow answer must not paint over a newer one), and the
//! one place a refusal is turned into words — so a 401 anywhere leads to signing
//! in, and every other failure says what happened and what to do next.

import { el, fill } from "./dom.ts";
import type { Capabilities } from "./models.ts";

export type Failure = { readonly ok: false; readonly status: number; readonly code: string; readonly message: string };

export type Screen = {
  readonly main: HTMLElement;
  /** Whether this render is still the one on screen. */
  readonly live: () => boolean;
  /** Shows the sign-in form; after signing in the same view is drawn again. */
  readonly signIn: () => void;
  /** Draws this view again. */
  readonly redraw: () => void;
  /** What the signed-in key may do, as the server answered; null when it could not be asked. Offers only — the
   * server decides. */
  readonly may: Capabilities | null;
};

/** What to tell the operator about `failure`, beyond the server's own message. */
function next(failure: Failure): string {
  switch (failure.code) {
    case "forbidden":
      return "Your key does not allow this. Ask an operator or your space's administrator.";
    case "rate_limit":
      return "Wait a minute, then try again.";
    case "unavailable":
    case "network":
      return "Try again in a moment.";
    case "precondition_failed":
      return "Reload it and check it before acting.";
    case "action_not_recorded":
      return "The action was carried out. Tell whoever audits this node; the server log holds the record.";
    default:
      return failure.status >= 500 ? "Try again; if it keeps failing, check the server log." : "";
  }
}

/** An inline message for a failed change: the server's words plus what to do next. */
export function refusal(failure: Failure): HTMLElement {
  const advice = next(failure);
  return el("p", { class: "message bad", role: "alert" }, el("strong", {}, "Not done. "), failure.message, advice === "" ? null : ` ${advice}`);
}

/** Replaces the view with `failure`, or with sign-in when the session is missing. */
export function failed(screen: Screen, title: string, failure: Failure): void {
  if (!screen.live()) {
    return;
  }
  if (failure.status === 401) {
    screen.signIn();
    return;
  }
  // A refusal of the key's authority does not change on a retry, so it offers none.
  const retry = failure.status === 403 ? null : el("button", { type: "button" }, "Try again");
  retry?.addEventListener("click", screen.redraw);
  fill(screen.main, head(title), el("div", { class: "card" }, refusal(failure), retry));
}

/** The view's heading, focusable so a route change can move focus to it. */
export const heading = (title: string): HTMLElement => el("h1", { tabindex: "-1", class: "view-title" }, title);

/** The top of a view: its heading, a line under it, and the view's primary action on the right. */
export const head = (title: string, lede?: Node | string | null, action?: Node | null): HTMLElement =>
  el("div", { class: "page-head" }, el("div", {}, heading(title), lede === undefined || lede === null ? null : el("p", { class: "lede" }, lede)), action ?? null);

/** The loading state: says what is loading, politely, without hiding the title. */
export function loading(screen: Screen, title: string, what: string): void {
  fill(screen.main, head(title), el("div", { class: "card", "aria-busy": "true" }, el("p", { class: "muted" }, `Loading ${what}…`)));
}

/** An empty state: why it is empty and what to do. */
export const empty = (why: string, action?: Node): HTMLElement => el("div", { class: "empty" }, el("p", {}, why), action ?? null);
