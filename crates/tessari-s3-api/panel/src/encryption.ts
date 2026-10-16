//! The encryption keys of this node, for an operator: whether they are held,
//! the form that unseals them with the keys passphrase (the first unseal
//! creates them), sealing them again, and the keys created by name that
//! objects can be sealed under with `aws:kms`. Each node holds its own keys,
//! so this acts on the node serving the console; other nodes hold a new named
//! key after their next unseal. The server wants a recent sign-in and records
//! every attempt without the passphrase.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { readKeys, type Keys } from "./encryption-model.ts";
import { refusal, type Failure, type Screen } from "./screen.ts";

const BASE = "/encryption";

function done(screen: Screen, failure: Failure | null, said: string): Failure | null {
  if (failure === null) {
    announce(said);
    screen.redraw();
    return null;
  }
  if (failure.status === 401) {
    screen.signIn();
    return null;
  }
  return failure;
}

function stateLine(keys: Keys): HTMLElement {
  switch (keys.state) {
    case "uninitialised":
      return el("p", {}, el("span", { class: "chip warn" }, "Not created"), " The first unseal creates the keys under the passphrase you give.");
    case "sealed":
      return el("p", {}, el("span", { class: "chip bad" }, "Sealed"), " This node cannot read or write encrypted objects until the keys are unsealed.");
    case "unsealed":
      return el("p", {}, el("span", { class: "chip ok" }, "Unsealed"), " New objects are wrapped under ", mono(keys.active), `; ${keys.keys.toLocaleString()} held.`);
  }
}

function unsealForm(screen: Screen, keys: Keys): HTMLElement {
  const first = keys.state === "uninitialised";
  const hint = first
    ? "At least 15 characters, without quotes or backslashes. It becomes the keys passphrase: there is no recovering it."
    : "The keys passphrase. Wrong ones in a row make the next try wait.";
  const passphrase = field("keys-passphrase", "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, hint);
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": "keys-unseal-title" },
    el("p", { id: "keys-unseal-title" }, el("strong", {}, first ? "Create and unseal the keys" : "Unseal the keys")),
    passphrase.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Unseal")),
    said,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const answer = await call("POST", `${BASE}/unseal`, readKeys, { passphrase: passphrase.input.value });
    passphrase.input.value = "";
    const failure = done(screen, answer.ok ? null : answer, "The encryption keys are unsealed on this node.");
    if (failure !== null) {
      fill(said, refusal(failure));
    }
  });
  return form;
}

function namedKeys(keys: readonly string[]): HTMLElement {
  if (keys.length === 0) {
    return el("p", {}, "No named keys yet. A request asking for ", mono("aws:kms"), " without a key id uses the key named ", mono("aws-s3"), ".");
  }
  return el("p", {}, "Named keys: ", ...keys.flatMap((name, at) => (at === 0 ? [mono(name)] : [", ", mono(name)])), ".");
}

function createForm(screen: Screen): HTMLElement {
  const name = field("key-name", "Key name", { autocomplete: "off", required: "", pattern: "[a-z0-9-]{1,64}" }, "1 to 64 of a-z, 0-9 and -. Names are visible to anyone who can list the keys vault; keys are never replaced.");
  const passphrase = field("key-passphrase", "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, "Opens the keys vault for this one write.");
  const why = field("key-reason", "Reason", { autocomplete: "off" }, "Kept in the action record.");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm card-part", novalidate: "", "aria-labelledby": "keys-create-title" },
    el("p", { id: "keys-create-title" }, el("strong", {}, "Create a named key")),
    name.row,
    passphrase.row,
    why.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create")),
    said,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const reason = why.input.value.trim();
    const answer = await call("POST", `${BASE}/keys`, readKeys, {
      name: name.input.value.trim(),
      passphrase: passphrase.input.value,
      ...(reason === "" ? {} : { reason }),
    });
    passphrase.input.value = "";
    const failure = done(screen, answer.ok ? null : answer, "The key is created and held on this node.");
    if (failure !== null) {
      fill(said, refusal(failure));
    }
  });
  return form;
}

function sealButton(screen: Screen, region: HTMLElement): HTMLElement {
  const button = el("button", { type: "button", class: "danger" }, "Seal…");
  button.addEventListener("click", () =>
    fill(
      region,
      confirmation(
        "keys-seal",
        el("span", {}, "the encryption keys on this node"),
        "This node forgets its keys: encrypted objects are refused here until someone unseals them with the passphrase.",
        "Seal",
        async (reason) => {
          const answer = await call("POST", `${BASE}/seal`, readKeys, { reason });
          return done(screen, answer.ok ? null : answer, "The encryption keys are sealed on this node.");
        },
        () => fill(region),
      ),
    ),
  );
  return button;
}

/** The card, for an operator; `null` for anybody else, whom the server refuses. */
export async function encryptionCard(screen: Screen): Promise<HTMLElement | null> {
  if (screen.may?.operate !== true) {
    return null;
  }
  const card = el("section", { class: "card" }, el("h2", {}, "Encryption keys"));
  const answer = await call("GET", BASE, readKeys);
  if (!answer.ok) {
    card.append(refusal(answer));
    return card;
  }
  const keys = answer.value;
  const region = el("div", {});
  card.append(stateLine(keys));
  if (keys.state === "unsealed") {
    card.append(namedKeys(keys.named), el("div", { class: "actions" }, sealButton(screen, region)), region, createForm(screen));
  } else {
    card.append(unsealForm(screen, keys));
  }
  return card;
}
