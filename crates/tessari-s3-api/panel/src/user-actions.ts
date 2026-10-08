//! A user row's actions, each opening an inline region under the row: issuing
//! a key (its secret is shown here once and never again — the server keeps it
//! sealed and will not hand it back), listing and revoking keys, disabling or
//! enabling the user, and granting or removing access to one bucket. Every
//! change takes a reason.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { iconAction } from "./icon-action.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { readIssue } from "./keys-model.ts";
import { ignored, type IssuedKey, type User } from "./models.ts";
import { open, settle } from "./row-region.ts";
import { refusal, type Failure, type Screen } from "./screen.ts";
import { keys } from "./user-keys.ts";

/** The one view of a new key's secret: copy it now, then dismiss it. */
export function shownOnce(key: IssuedKey, done: () => void): HTMLElement {
  const copy = el("button", { type: "button" }, "Copy secret");
  const finish = el("button", { type: "button", class: "primary" }, "I have stored it");
  const said = el("p", { class: "hint", "aria-live": "polite" });
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(key.secret_access_key);
      said.textContent = "Copied.";
    } catch {
      said.textContent = "Copying is not allowed here; select the secret and copy it by hand.";
    }
  });
  finish.addEventListener("click", done);
  queueMicrotask(() => copy.focus());
  return el(
    "div",
    { class: "once", role: "alert" },
    el("p", {}, el("strong", {}, "Store this secret now. "), "It is shown once; the server will not show it again."),
    el("dl", {}, el("dt", {}, "Access key id"), el("dd", {}, mono(key.access_key_id)), el("dt", {}, "Secret access key"), el("dd", {}, mono(key.secret_access_key))),
    el("div", { class: "actions" }, copy, finish),
    said,
  );
}

/** A repeated submission's answer: the key exists, and its secret was shown once already. */
function alreadyIssued(id: string, done: () => void): HTMLElement {
  const finish = el("button", { type: "button", class: "primary" }, "Close");
  finish.addEventListener("click", done);
  queueMicrotask(() => finish.focus());
  return el(
    "div",
    { class: "once", role: "status" },
    el("p", {}, "This form already issued key ", mono(id), ". Its secret was shown once and cannot be shown again; if it was not stored, revoke the key and issue a new one."),
    el("div", { class: "actions" }, finish),
  );
}

function issueKey(screen: Screen, user: User, line: HTMLTableRowElement, opener: HTMLButtonElement): void {
  const region = el("div", {});
  const close = open(line, region);
  const back = (): void => {
    close();
    opener.focus();
  };
  // Minted as the form opens and sent with every submission of it, so a submission repeated after a lost answer
  // gets the key this form already issued rather than a second one.
  const token = crypto.randomUUID();
  // The secret is on screen once it arrives; a repeated submission's later "already issued" must not cover it.
  let secretShown = false;
  fill(
    region,
    confirmation(
      `key-${user.name}`,
      mono(user.name),
      "A new access key for this user. Its secret is shown once, right here.",
      "Issue key",
      async (reason) => {
        const answer = await call("POST", `/users/${encodeURIComponent(user.name)}/keys`, readIssue, { reason, idempotency_key: token });
        if (!answer.ok) {
          return settle(screen, answer);
        }
        const issued = answer.value;
        if (issued.kind === "already") {
          if (secretShown) {
            return null;
          }
          announce(`This form already issued ${user.name} a key.`);
          fill(region, alreadyIssued(issued.access_key_id, back));
          return null;
        }
        secretShown = true;
        announce(`Issued a key for ${user.name}. Store its secret now.`);
        fill(region, shownOnce(issued.key, () => {
          fill(region);
          back();
        }));
        return null;
      },
      back,
    ),
  );
}

function setDisabled(screen: Screen, user: User, line: HTMLTableRowElement, opener: HTMLButtonElement): void {
  const disable = !user.disabled;
  const region = el("div", {});
  const close = open(line, region);
  const back = (): void => {
    close();
    opener.focus();
  };
  fill(
    region,
    confirmation(
      `state-${user.name}`,
      mono(user.name),
      disable ? "Every key of this user stops working within five seconds, on every node." : "The user's keys work again within five seconds.",
      disable ? "Disable user" : "Enable user",
      async (reason) => {
        const answer = await call("PUT", `/users/${encodeURIComponent(user.name)}/disabled`, ignored, { disabled: disable, reason });
        if (!answer.ok) {
          return settle(screen, answer);
        }
        announce(`${disable ? "Disabled" : "Enabled"} ${user.name}.`);
        screen.redraw();
        return null;
      },
      back,
    ),
  );
}

function grants(screen: Screen, user: User, line: HTMLTableRowElement, opener: HTMLButtonElement): void {
  const id = `grant-${user.name}`;
  const bucket = field(`${id}-bucket`, "Bucket", { spellcheck: "false", autocomplete: "off", required: "" }, `A bucket of space ${user.space}.`);
  const read = el("input", { id: `${id}-read`, type: "checkbox", checked: "" });
  const write = el("input", { id: `${id}-write`, type: "checkbox" });
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const grant = el("button", { type: "submit", class: "primary" }, "Grant");
  const remove = el("button", { type: "button", class: "danger" }, "Remove grant");
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Bucket access: "), mono(user.name)),
    bucket.row,
    el("fieldset", { class: "checks" }, el("legend", {}, "Access"), el("label", { class: "check", for: read.id }, read, "Read"), el("label", { class: "check", for: write.id }, write, "Write")),
    reason.row,
    el("div", { class: "actions" }, grant, remove, cancel),
    said,
  );
  const close = open(line, form);
  const back = (): void => {
    close();
    opener.focus();
  };
  const route = (): string => `/users/${encodeURIComponent(user.name)}/grants/${encodeURIComponent(bucket.input.value.trim())}`;
  const done = (words: string): void => {
    announce(words);
    back();
  };
  const show = (failure: Failure): void => {
    const shown = settle(screen, failure);
    if (shown !== null) {
      fill(said, refusal(shown));
    }
  };
  cancel.addEventListener("click", back);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      back();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const answer = await call("PUT", route(), ignored, { read: read.checked, write: write.checked, reason: reason.input.value.trim() });
    if (answer.ok) {
      done(`Granted ${user.name} access to ${bucket.input.value.trim()}.`);
    } else {
      show(answer);
    }
  });
  remove.addEventListener("click", async () => {
    const answer = await call("DELETE", route(), ignored, { reason: reason.input.value.trim() });
    if (answer.ok) {
      done(`Removed ${user.name}'s grant on ${bucket.input.value.trim()}.`);
    } else {
      show(answer);
    }
  });
  queueMicrotask(() => bucket.input.focus());
}

/** The buttons for `user`'s row. */
export function rowActions(screen: Screen, user: User, line: HTMLTableRowElement): HTMLButtonElement[] {
  const key = iconAction("key", "Issue key…", `Issue a key to ${user.name}`);
  const list = iconAction("keys", "Keys…", `Keys of ${user.name}`);
  const state = user.disabled ? iconAction("check", "Enable…", `Enable ${user.name}`) : iconAction("ban", "Disable…", `Disable ${user.name}`);
  const access = iconAction("shield", "Bucket access…", `Bucket access of ${user.name}`);
  key.addEventListener("click", () => issueKey(screen, user, line, key));
  list.addEventListener("click", () => keys(screen, user, line, list));
  state.addEventListener("click", () => setDisabled(screen, user, line, state));
  access.addEventListener("click", () => grants(screen, user, line, access));
  return [key, list, state, access];
}
