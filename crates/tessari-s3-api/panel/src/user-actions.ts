//! A user row's actions, each opening an inline region under the row: issuing
//! a key (its secret is shown here once and never again — the server keeps it
//! sealed and will not hand it back), disabling or enabling the user, and
//! granting or removing access to one bucket. Every one takes a reason.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { ignored, readIssued, type IssuedKey, type User } from "./models.ts";
import { refusal, type Failure, type Screen } from "./screen.ts";

const COLUMNS = "6";

/** The failure to show, after handing a lost session to sign-in. */
function settle(screen: Screen, failure: Failure): Failure | null {
  if (failure.status === 401) {
    screen.signIn();
    return null;
  }
  return failure;
}

/** Opens `region` under `line`, replacing any region already open there. */
function open(line: HTMLTableRowElement, region: HTMLElement): () => void {
  const next = line.nextElementSibling;
  if (next instanceof HTMLTableRowElement && next.classList.contains("asking")) {
    next.remove();
  }
  const ask = el("tr", { class: "asking" }, el("td", { colspan: COLUMNS }, region));
  line.after(ask);
  return () => ask.remove();
}

/** The one view of a new key's secret: copy it now, then dismiss it. */
function shownOnce(key: IssuedKey, done: () => void): HTMLElement {
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

function issueKey(screen: Screen, user: User, line: HTMLTableRowElement, opener: HTMLButtonElement): void {
  const region = el("div", {});
  const close = open(line, region);
  const back = (): void => {
    close();
    opener.focus();
  };
  fill(
    region,
    confirmation(
      `key-${user.name}`,
      mono(user.name),
      "A new access key for this user. Its secret is shown once, right here.",
      "Issue key",
      async (reason) => {
        const answer = await call("POST", `/users/${encodeURIComponent(user.name)}/keys`, readIssued, { reason });
        if (!answer.ok) {
          return settle(screen, answer);
        }
        announce(`Issued a key for ${user.name}. Store its secret now.`);
        fill(region, shownOnce(answer.value, () => {
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
  const key = el("button", { type: "button", class: "quiet" }, "Issue key…");
  const state = el("button", { type: "button", class: "quiet" }, user.disabled ? "Enable…" : "Disable…");
  const access = el("button", { type: "button", class: "quiet" }, "Bucket access…");
  key.addEventListener("click", () => issueKey(screen, user, line, key));
  state.addEventListener("click", () => setDisabled(screen, user, line, state));
  access.addEventListener("click", () => grants(screen, user, line, access));
  return [key, state, access];
}
