//! The settings every node of this region shares, for an operator: each with
//! its stored value — a secret only as stored or not — when a change applies,
//! and a form to change it against the value read, with a reason; a secret
//! takes the keys passphrase. The server checks the value with the node's own
//! configuration and records every attempt without the value.

import { call } from "./api.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { ignored } from "./models.ts";
import { readSettings, type Setting } from "./settings-model.ts";
import { refusal, type Screen } from "./screen.ts";

const BASE = "/settings";
const PREFIX = "TESSARIDB_S3_";

const short = (name: string): string => (name.startsWith(PREFIX) ? name.slice(PREFIX.length) : name);

function valueCell(setting: Setting): Node | string {
  if (setting.secret) {
    return setting.stored ? el("span", { class: "chip ok" }, "Stored") : "Not stored";
  }
  return setting.value === null ? "Not stored" : mono(setting.value);
}

function changeForm(screen: Screen, setting: Setting, region: HTMLElement): HTMLElement {
  const id = `setting-${short(setting.name).toLowerCase()}`;
  const value = field(`${id}-value`, short(setting.name), { autocomplete: "off", required: "", ...(setting.secret ? { type: "password" } : {}) }, setting.live ? "Applies on every node within 5 seconds." : "Applies when each node next starts.");
  const why = field(`${id}-reason`, "Reason", { autocomplete: "off", required: "" }, "Kept in the action record; the value is not.");
  const passphrase = setting.secret ? field(`${id}-passphrase`, "Keys passphrase", { type: "password", autocomplete: "off", required: "" }, "Opens the secrets vault for this one write.") : null;
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Change "), mono(setting.name)),
    value.row,
    passphrase === null ? null : passphrase.row,
    why.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Change"), el("button", { type: "button", class: "quiet", "data-cancel": "" }, "Cancel")),
    said,
  );
  form.querySelector("[data-cancel]")?.addEventListener("click", () => fill(region));
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const answer = await call("PUT", `${BASE}/${setting.name}`, ignored, {
      value: value.input.value,
      expected: setting.value,
      reason: why.input.value.trim(),
      ...(passphrase === null || passphrase.input.value === "" ? {} : { passphrase: passphrase.input.value }),
    });
    if (passphrase !== null) {
      passphrase.input.value = "";
    }
    if (setting.secret) {
      value.input.value = "";
    }
    if (answer.ok) {
      announce(`${short(setting.name)} is changed.`);
      screen.redraw();
    } else if (answer.status === 401) {
      screen.signIn();
    } else {
      fill(said, refusal(answer));
    }
  });
  return form;
}

function action(screen: Screen, setting: Setting, region: HTMLElement): Node | string {
  if (setting.fixed) {
    return el("span", { class: "chip" }, "Fixed");
  }
  const button = el("button", { type: "button", class: "quiet" }, "Change…");
  button.addEventListener("click", () => fill(region, changeForm(screen, setting, region)));
  return button;
}

/** The card, for an operator; `null` for anybody else, whom the server refuses. */
export async function settingsCard(screen: Screen): Promise<HTMLElement | null> {
  if (screen.may?.operate !== true) {
    return null;
  }
  const card = el("section", { class: "card" }, el("h2", {}, "Settings"));
  const answer = await call("GET", BASE, readSettings);
  if (!answer.ok) {
    card.append(refusal(answer));
    return card;
  }
  const region = el("div", {});
  const rows = answer.value.settings.map((setting) =>
    row(mono(short(setting.name)), setting.scope === "global" ? "Every region" : answer.value.region, valueCell(setting), setting.live ? "Live" : "At start", action(screen, setting, region)),
  );
  card.append(
    el("p", {}, "Kept in the metadata store and shared by every node of ", mono(answer.value.region), ". A value set here wins over a node's environment."),
    table("Stored settings", ["Setting", "Applies to", "Value", "Takes effect", ""], rows),
    region,
  );
  return card;
}
