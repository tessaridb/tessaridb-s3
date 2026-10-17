//! The two halves of a group's page: its members and the buckets it is
//! granted. Each lists what is there with a remove action per row, and carries
//! a form to add one more; every change takes a reason. A member of the group's
//! space and a bucket of that space are all the server accepts — anything else
//! is refused as missing, so a group never reaches across spaces.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import type { GroupDetail, GroupGrant } from "./groups-model.ts";
import { iconAction } from "./icon-action.ts";
import { ignored } from "./models.ts";
import { settle } from "./row-region.ts";
import { empty, refusal, type Failure, type Screen } from "./screen.ts";

/** Where a group's changes go: `/spaces/{space}/groups/{group}` plus `rest`. */
export type GroupPath = (rest: string) => string;

/** A card with a heading, its table or empty state, a region for an open confirmation, and its add form. */
function card(id: string, title: string, body: HTMLElement, region: HTMLElement, form: HTMLElement): HTMLElement {
  return el("section", { class: "card", "aria-labelledby": `${id}-title` }, el("h2", { id: `${id}-title` }, title), body, region, form);
}

/** An add form: `fields`, a reason, and a submit that answers null when done or the failure to show. */
function addForm(id: string, button: string, fields: readonly HTMLElement[], act: (reason: string) => Promise<Failure | null>): HTMLElement {
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const said = el("div", { "aria-live": "polite" });
  const go = el("button", { type: "submit", class: "primary" }, button);
  const form = el("form", { class: "card-part", novalidate: "" }, ...fields, reason.row, el("div", { class: "actions" }, go), said);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    go.disabled = true;
    const failure = await act(reason.input.value.trim());
    go.disabled = false;
    if (failure !== null) {
      fill(said, refusal(failure));
    }
  });
  return form;
}

/** Opens a removal confirmation for `what` in `region`; on success the view is drawn again. */
function askRemoval(screen: Screen, region: HTMLElement, id: string, what: string, consequence: string, button: string, route: string, opener: HTMLButtonElement, done: string): void {
  const back = (): void => {
    fill(region);
    opener.focus();
  };
  fill(
    region,
    confirmation(
      id,
      mono(what),
      consequence,
      button,
      async (reason) => {
        const answer = await call("DELETE", route, ignored, { reason });
        if (!answer.ok) {
          return settle(screen, answer);
        }
        announce(done);
        screen.redraw();
        return null;
      },
      back,
    ),
  );
}

export function members(screen: Screen, group: GroupDetail, at: GroupPath): HTMLElement {
  const region = el("div", {});
  const line = (user: string): HTMLTableRowElement => {
    const remove = iconAction("close", "Remove…", `Remove ${user} from ${group.name}`, "danger");
    remove.addEventListener("click", () =>
      askRemoval(screen, region, `member-${user}`, user, `${user} loses what this group is granted within five seconds; its own grants stay.`, "Remove member", at(`/members/${encodeURIComponent(user)}`), remove, `Removed ${user} from ${group.name}.`),
    );
    return row(mono(user), el("div", { class: "actions" }, remove));
  };
  const user = field("add-member", "User name", { spellcheck: "false", autocomplete: "off", required: "" }, "A user of this group's space.");
  const form = addForm("add-member", "Add member", [user.row], async (reason) => {
    const name = user.input.value.trim();
    const answer = await call("PUT", at(`/members/${encodeURIComponent(name)}`), ignored, { reason });
    if (!answer.ok) {
      return settle(screen, answer);
    }
    announce(`Added ${name} to ${group.name}.`);
    screen.redraw();
    return null;
  });
  const body = group.members.length === 0 ? empty("No members yet. Add a user of this space below.") : table(`Members of ${group.name}`, ["User", "Actions"], group.members.map(line));
  return card("members", `Members (${group.members.length.toLocaleString()})`, body, region, form);
}

const yes = (granted: boolean): HTMLElement => el("span", { class: granted ? "chip ok" : "chip" }, granted ? "Yes" : "No");

export function grants(screen: Screen, group: GroupDetail, at: GroupPath): HTMLElement {
  const region = el("div", {});
  const line = (grant: GroupGrant): HTMLTableRowElement => {
    const remove = iconAction("close", "Remove…", `Remove ${group.name}'s grant on ${grant.bucket}`, "danger");
    remove.addEventListener("click", () =>
      askRemoval(screen, region, `grant-${grant.bucket}`, grant.bucket, "Every member loses this group's access to the bucket within five seconds; their own grants stay.", "Remove grant", at(`/grants/${encodeURIComponent(grant.bucket)}`), remove, `Removed ${group.name}'s grant on ${grant.bucket}.`),
    );
    return row(mono(grant.bucket), yes(grant.read), yes(grant.write), el("div", { class: "actions" }, remove));
  };
  const bucket = field("grant-bucket", "Bucket", { spellcheck: "false", autocomplete: "off", required: "" }, "A bucket of this group's space. Granting it again replaces what the grant allows.");
  const read = el("input", { id: "grant-read", type: "checkbox" });
  const write = el("input", { id: "grant-write", type: "checkbox" });
  read.checked = true;
  const access = el("fieldset", { class: "checks" }, el("legend", {}, "Access"), el("label", { class: "check", for: read.id }, read, "Read"), el("label", { class: "check", for: write.id }, write, "Write"));
  const form = addForm("grant", "Grant bucket", [bucket.row, access], async (reason) => {
    const name = bucket.input.value.trim();
    const answer = await call("PUT", at(`/grants/${encodeURIComponent(name)}`), ignored, { read: read.checked, write: write.checked, reason });
    if (!answer.ok) {
      return settle(screen, answer);
    }
    announce(`Granted ${group.name} access to ${name}.`);
    screen.redraw();
    return null;
  });
  const body = group.grants.length === 0 ? empty("No buckets granted yet. Grant one below.") : table(`Buckets granted to ${group.name}`, ["Bucket", "Read", "Write", "Actions"], group.grants.map(line));
  return card("grants", `Bucket grants (${group.grants.length.toLocaleString()})`, body, region, form);
}
