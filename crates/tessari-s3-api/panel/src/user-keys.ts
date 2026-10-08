//! A user's access keys, listed in a region under the user's row — never a
//! secret, which the server does not hand back — each revocable. Revoking names
//! the key, cannot be undone, and takes a reason; every node refuses the key
//! within five seconds.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { readKeys, type ListedKey } from "./keys-model.ts";
import { ignored, type User } from "./models.ts";
import { open, settle } from "./row-region.ts";
import { refusal, type Screen } from "./screen.ts";

/** What a key is for: every bucket the user reaches, or one upload. */
function reach(key: ListedKey): Node | string {
  if (key.upload === null) {
    return "Full";
  }
  const until = new Date(key.upload.expires * 1000).toISOString();
  return el("span", {}, "Upload of ", mono(`${key.upload.bucket}/${key.upload.key}`), ` until ${moment(until)}`);
}

export function keys(screen: Screen, user: User, line: HTMLTableRowElement, opener: HTMLButtonElement): void {
  const region = el("div", { class: "keys", "aria-live": "polite" });
  const close = open(line, region);
  const back = (): void => {
    close();
    opener.focus();
  };
  const route = `/users/${encodeURIComponent(user.name)}/keys`;
  const done = el("button", { type: "button", class: "quiet" }, "Close");
  done.addEventListener("click", back);
  const revoke = (key: ListedKey, button: HTMLButtonElement): void => {
    const ask = el("div", {});
    button.after(ask);
    fill(
      ask,
      confirmation(
        `revoke-${key.access_key_id}`,
        mono(key.access_key_id),
        `Every node refuses this key of ${user.name} within five seconds. It cannot be restored; a new key has a new secret.`,
        "Revoke key",
        async (reason) => {
          const answer = await call("DELETE", `${route}/${encodeURIComponent(key.access_key_id)}`, ignored, { reason });
          if (!answer.ok) {
            return settle(screen, answer);
          }
          announce(`Revoked key ${key.access_key_id} of ${user.name}.`);
          void show();
          return null;
        },
        () => {
          ask.remove();
          button.focus();
        },
      ),
    );
  };
  const show = async (): Promise<void> => {
    fill(region, el("p", { class: "muted" }, `Loading the keys of ${user.name}…`));
    const answer = await call("GET", route, readKeys);
    if (!answer.ok) {
      const shown = settle(screen, answer);
      if (shown !== null) {
        fill(region, refusal(shown), done);
      }
      return;
    }
    if (answer.value.length === 0) {
      fill(region, el("p", {}, `${user.name} has no access keys.`), done);
      return;
    }
    const rows = answer.value.map((key) => {
      const button = el("button", { type: "button", class: "danger" }, "Revoke…");
      button.addEventListener("click", () => revoke(key, button));
      return row(mono(key.access_key_id), moment(key.created), key.disabled ? "Disabled" : "Active", reach(key), button);
    });
    fill(region, table(`Access keys of ${user.name}`, ["Access key id", "Issued", "State", "Reach", ""], rows), done);
  };
  void show();
}
