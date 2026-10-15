//! The action record, newest first: who did what to which bucket or object,
//! when, why, and how it ended. Paged by position; there is no total.

import { call, search } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { readActions } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

const TITLE = "Action record";
const PAGE = 50;

export async function actions(screen: Screen, before: number | null): Promise<void> {
  loading(screen, TITLE, "the action record");
  const answer = await call("GET", `/actions${search([["before", before], ["limit", PAGE]])}`, readActions);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const page = answer.value;
  const rows = page.actions.map((action) =>
    row(
      moment(action.at),
      mono(action.operator),
      action.operation.replaceAll("_", " "),
      mono(action.target),
      action.reason ?? el("span", { class: "muted" }, "none given"),
      el("span", { class: `chip ${action.outcome === "done" || action.outcome === "sent" ? "ok" : "warn"}` }, action.outcome.replaceAll("_", " ")),
    ),
  );
  fill(
    screen.main,
    head(TITLE, "Every change and every download made through this console, kept for a year and never edited."),
    el(
      "section",
      { class: "card flush" },
      rows.length === 0
        ? empty(before === null ? "No console actions are recorded yet." : "No older actions.")
        : table("Console actions, newest first", ["When", "Operator", "Operation", "Target", "Reason", "Outcome"], rows),
      before === null && page.next === null
        ? null
        : el(
        "p",
        { class: "paging" },
        before === null ? null : el("a", { href: format({ kind: "actions", before: null }) }, "Newest"),
        before !== null && page.next !== null ? " · " : null,
        page.next === null ? null : el("a", { href: format({ kind: "actions", before: page.next }) }, "Older"),
      ),
    ),
  );
}
