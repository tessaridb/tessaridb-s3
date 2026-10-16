//! The encryption keys of this node, for an operator: whether they are held,
//! the form that unseals them with the keys passphrase (the first unseal
//! creates them), and sealing them again. Each node holds its own keys, so
//! this acts on the node serving the console. The server wants a recent
//! sign-in and records every attempt without the passphrase.

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
    card.append(el("div", { class: "actions" }, sealButton(screen, region)), region);
  } else {
    card.append(unsealForm(screen, keys));
  }
  return card;
}
