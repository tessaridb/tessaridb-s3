//! The listing's side pane: one selected object's metadata beside the list, so
//! the operator keeps their place while looking. It reads the object's record
//! only — the bytes stay on the drives until somebody downloads them.

import { call, search } from "./api.ts";
import { el, fill, mono } from "./dom.ts";
import { iconAction } from "./icon-action.ts";
import { readDetail } from "./models.ts";
import { facts, pairs } from "./object-facts.ts";
import { format } from "./route.ts";
import type { Screen } from "./screen.ts";

/** Draws `key`'s metadata into `pane`; `close` takes the selection away. Stale answers (a newer selection, another view) are dropped by `current`. */
export async function describe(screen: Screen, pane: HTMLElement, bucket: string, key: string, close: () => void, current: () => boolean): Promise<void> {
  const name = key.slice(key.lastIndexOf("/") + 1) || key;
  const shut = iconAction("close", "Close", `Close the details of ${name}`);
  shut.addEventListener("click", close);
  const top = el("div", {}, el("div", { class: "pane-head" }, el("h2", { id: "pane-title" }, "Details"), shut), el("p", { class: "pane-key" }, mono(key)));
  fill(pane, top, el("p", { class: "pane-note", role: "status" }, "Loading the details…"));
  const answer = await call("GET", `/buckets/${encodeURIComponent(bucket)}/object${search([["key", key]])}`, readDetail);
  if (!screen.live() || !current()) {
    return;
  }
  if (!answer.ok) {
    if (answer.status === 401) {
      screen.signIn();
      return;
    }
    fill(pane, top, el("p", { class: "pane-note", role: "alert" }, answer.status === 404 ? "This object is gone; reload the listing." : answer.message));
    return;
  }
  const detail = answer.value;
  fill(
    pane,
    top,
    facts(detail),
    el("p", { class: "pane-link" }, el("a", { href: format({ kind: "object", bucket, key }) }, "Open the object's page"), " to download or delete it."),
    el("h3", {}, "Headers"),
    pairs("Stored headers", detail.headers),
    el("h3", {}, "User metadata"),
    pairs("User metadata", detail.metadata),
    el("h3", {}, "Tags"),
    pairs("Tags", detail.tags),
    el("h3", {}, "Checksums"),
    pairs("Stored checksums", detail.checksums),
  );
}
