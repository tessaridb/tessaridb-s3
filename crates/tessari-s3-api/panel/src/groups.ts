//! Groups: a space's named sets of users that buckets are granted to, so a
//! grant reaches every member at once. An operator first picks the space; a
//! space admin lands on its own. The server judges every route as managing the
//! space and answers alike for a space that is not the caller's and one that
//! does not exist; this page only offers. A new group takes a reason, which
//! goes into the action record.

import { call } from "./api.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { readGroups } from "./groups-model.ts";
import { icon } from "./icons.ts";
import { ignored, readSpaces } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, refusal, type Screen } from "./screen.ts";

const TITLE = "Groups";

const groupsOf = (space: string): string => `/spaces/${encodeURIComponent(space)}/groups`;

function createForm(screen: Screen, space: string, opener: HTMLButtonElement): HTMLElement {
  const name = field("new-group", "Group name", { spellcheck: "false", autocomplete: "off", required: "" }, "1-63 lowercase letters, digits and inner hyphens; unique in the space.");
  const reason = field("new-group-reason", "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const said = el("div", { "aria-live": "polite" });
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const form = el(
    "form",
    { id: "new-group-form", class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-group-title" },
    el("h2", { id: "new-group-title" }, "New group in ", mono(space)),
    name.row,
    reason.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create group"), cancel),
    said,
  );
  const close = (): void => {
    form.hidden = true;
    opener.setAttribute("aria-expanded", "false");
    opener.focus();
  };
  opener.addEventListener("click", () => {
    form.hidden = false;
    opener.setAttribute("aria-expanded", "true");
    name.input.focus();
  });
  cancel.addEventListener("click", close);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const wanted = name.input.value.trim();
    const answer = await call("POST", groupsOf(space), ignored, { name: wanted, reason: reason.input.value.trim() });
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
      return;
    }
    announce(`Created group ${wanted} in ${space}.`);
    screen.redraw();
  });
  return form;
}

/** The groups of `space`. */
async function ofSpace(screen: Screen, space: string): Promise<void> {
  loading(screen, TITLE, "groups");
  const answer = await call("GET", groupsOf(space), readGroups);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-group-form" }, icon("plus"), "New group");
  const link = (group: string): HTMLElement => el("a", { href: format({ kind: "group", space, group }) }, mono(group));
  fill(
    screen.main,
    head(TITLE, el("span", {}, `${answer.value.length.toLocaleString()} in space `, mono(space)), opener),
    createForm(screen, space, opener),
    el(
      "section",
      { class: "card flush" },
      answer.value.length === 0
        ? empty("No groups in this space yet. Create one with “New group”, add users to it, then grant it buckets.")
        : table(`Groups of ${space}`, ["Name", "Created"], answer.value.map((group) => row(link(group.name), moment(group.created)))),
    ),
  );
}

/** The spaces an operator may pick from. */
async function pick(screen: Screen): Promise<void> {
  loading(screen, TITLE, "spaces");
  const answer = await call("GET", "/spaces", readSpaces);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const link = (space: string): HTMLElement => el("a", { href: format({ kind: "groups", space }) }, mono(space));
  fill(
    screen.main,
    head(TITLE, "Pick the space whose groups to manage."),
    el(
      "section",
      { class: "card flush" },
      answer.value.length === 0
        ? empty("No spaces yet. Create one under Spaces first.")
        : table("Spaces", ["Space", "Created"], answer.value.map((space) => row(link(space.name), moment(space.created)))),
    ),
  );
}

export function groups(screen: Screen, space: string | null): Promise<void> {
  if (space !== null) {
    return ofSpace(screen, space);
  }
  // A space admin manages one space, its own; an operator, every space.
  const own = screen.may?.operate === true ? null : (screen.may?.space ?? null);
  return own === null ? pick(screen) : ofSpace(screen, own);
}
