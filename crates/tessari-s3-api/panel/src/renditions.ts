//! Renditions that failed for good: the tasks whose five hand-outs are spent,
//! with the original, the output, and the last error a worker recorded. They
//! stay listed until the original is written again; for operators.

import { call } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { readFailedRenditions } from "./models.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

const TITLE = "Renditions";

export async function renditions(screen: Screen): Promise<void> {
  loading(screen, TITLE, "failed renditions");
  const answer = await call("GET", "/media/failed", readFailedRenditions);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const page = answer.value;
  const rows = page.failed.map((one) =>
    row(
      mono(`${one.bucket}/${one.key}`),
      one.version_id === null ? el("span", { class: "muted" }, "current") : mono(one.version_id),
      mono(one.output),
      el("span", { class: "numeric" }, String(one.attempts)),
      one.error ?? el("span", { class: "muted" }, "none recorded"),
    ),
  );
  fill(
    screen.main,
    head(TITLE, "Renditions the workers stopped trying after five attempts, with the last error. Writing the original again plans it anew."),
    el(
      "section",
      { class: "card flush" },
      rows.length === 0
        ? empty("No rendition has failed.")
        : table("Failed renditions", ["Original", "Version", "Output", "Tries", "Last error"], rows),
      page.more ? el("p", { class: "paging" }, "Showing the first 100 of each queue.") : null,
    ),
  );
}
