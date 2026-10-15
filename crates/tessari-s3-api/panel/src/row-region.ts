//! The inline region a user row's action opens under the row, and how a failure
//! inside one is settled.

import { el } from "./dom.ts";
import type { Failure, Screen } from "./screen.ts";

const COLUMNS = "6";

/** The failure to show, after handing a lost session to sign-in. */
export function settle(screen: Screen, failure: Failure): Failure | null {
  if (failure.status === 401) {
    screen.signIn();
    return null;
  }
  return failure;
}

/** Opens `region` under `line`, replacing any region already open there. */
export function open(line: HTMLTableRowElement, region: HTMLElement): () => void {
  const next = line.nextElementSibling;
  if (next instanceof HTMLTableRowElement && next.classList.contains("asking")) {
    next.remove();
  }
  const ask = el("tr", { class: "asking" }, el("td", { colspan: COLUMNS }, region));
  line.after(ask);
  return () => ask.remove();
}

