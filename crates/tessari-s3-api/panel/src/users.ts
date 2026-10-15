//! Users: operators see every space's users, a space admin its own space's.
//! Creating a user is judged by the server on the user it would be, so a space
//! admin who asks for an administrator, an operator or a cluster viewer is told
//! the key does not allow it. Each row's actions are in `user-actions.ts`.

import { call } from "./api.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { icon } from "./icons.ts";
import { ignored, readUsers, type User } from "./models.ts";
import { empty, failed, head, loading, refusal, type Screen } from "./screen.ts";
import { rowActions } from "./user-actions.ts";

const TITLE = "Users";

function check(id: string, label: string): { readonly row: HTMLElement; readonly input: HTMLInputElement } {
  const input = el("input", { id, name: id, type: "checkbox" });
  return { row: el("label", { class: "check", for: id }, input, label), input };
}

function createForm(screen: Screen, opener: HTMLButtonElement): HTMLElement {
  const name = field("new-user", "User name", { spellcheck: "false", autocomplete: "off", required: "" }, "1-63 lowercase letters, digits and inner hyphens; unique across the store.");
  const space = field("new-user-space", "Space", { spellcheck: "false", autocomplete: "off", required: "" });
  const role = el("select", { id: "new-user-role" }, el("option", { value: "member" }, "Member"), el("option", { value: "space_admin" }, "Space admin"));
  const creates = check("new-user-creates", "May create buckets");
  const operates = check("new-user-operator", "Operator (every space)");
  const views = check("new-user-viewer", "Sees the cluster");
  const reason = field("new-user-reason", "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const said = el("div", { "aria-live": "polite" });
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const form = el(
    "form",
    { id: "new-user-form", class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-user-title" },
    el("h2", { id: "new-user-title" }, "New user"),
    name.row,
    space.row,
    el("div", { class: "field" }, el("label", { for: role.id }, "Role"), role),
    el("fieldset", { class: "checks" }, el("legend", {}, "Permissions"), creates.row, operates.row, views.row),
    reason.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create user"), cancel),
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
    const answer = await call("POST", "/users", ignored, {
      name: wanted,
      space: space.input.value.trim(),
      role: role.value,
      create_buckets: creates.input.checked,
      operator: operates.input.checked,
      cluster_viewer: views.input.checked,
      reason: reason.input.value.trim(),
    });
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
      return;
    }
    announce(`Created user ${wanted}. Issue a key so it can sign in.`);
    screen.redraw();
  });
  return form;
}

/** What a user may do, in words. */
function standing(user: User): string {
  const parts = [user.role === "space_admin" ? "space admin" : "member"];
  if (user.create_buckets) parts.push("creates buckets");
  if (user.operator) parts.push("operator");
  if (user.cluster_viewer) parts.push("sees the cluster");
  return parts.join(" · ");
}

function userRow(screen: Screen, user: User): HTMLTableRowElement {
  const state = el("span", { class: user.disabled ? "chip bad" : "chip ok" }, user.disabled ? "Disabled" : "Active");
  const buttons = el("div", { class: "actions" });
  const line = row(mono(user.name), mono(user.space), standing(user), state, moment(user.created), buttons);
  buttons.append(...rowActions(screen, user, line));
  return line;
}

export async function users(screen: Screen): Promise<void> {
  loading(screen, TITLE, "users");
  const answer = await call("GET", "/users", readUsers);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-user-form" }, icon("plus"), "New user");
  fill(
    screen.main,
    head(TITLE, `${answer.value.length.toLocaleString()} you administer`, opener),
    createForm(screen, opener),
    el(
      "section",
      { class: "card flush" },
      answer.value.length === 0
        ? empty("No users yet. Create one with “New user”, then issue it a key.")
        : table("Users", ["Name", "Space", "Standing", "State", "Created", "Actions"], answer.value.map((user) => userRow(screen, user))),
    ),
  );
}
