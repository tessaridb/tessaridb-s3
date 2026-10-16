//! A bucket's default encryption: how a write that asks for no encryption is
//! sealed — under the server's key, under a key an operator created by name, or
//! not at all. The server sets it only while this node holds its keys and, for a
//! named key, that key; its refusal is shown as it came. Who may configure the
//! bucket sets it; the reason goes into the action record.

import { call } from "./api.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { ignored, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";

const KEY_NAME = /^[a-z0-9-]{1,64}$/;

/** What the bucket list shows for a bucket's default encryption; nothing while it has none. */
export function encryptionChip(bucket: Bucket): HTMLElement | null {
  const sealing = bucket.encryption;
  if (sealing === null) {
    return null;
  }
  return el("span", { class: "chip ok", title: "Default encryption" }, sealing.algorithm === "AES256" ? "AES256" : `aws:kms · ${sealing.key}`);
}

/** The default-encryption form for `bucket`, as a region `open` places under its row; `close` removes it. */
export function encryptionForm(screen: Screen, bucket: Bucket, close: () => void): HTMLElement {
  const id = `sse-${bucket.name}`;
  const current = bucket.encryption?.algorithm ?? "none";
  const choice = (value: string, label: string, note: string): HTMLElement => {
    const input = el("input", { type: "radio", name: `${id}-algorithm`, id: `${id}-${value}`, value });
    if (current === value) {
      input.setAttribute("checked", "");
    }
    return el("div", { class: "choice" }, input, el("label", { for: `${id}-${value}` }, el("strong", {}, label), " — ", note));
  };
  const named = bucket.encryption?.algorithm === "aws:kms" ? bucket.encryption.key : "";
  const key = field(`${id}-key`, "Key name", { maxlength: "64", value: named, autocomplete: "off" }, "For aws:kms: a key created on the Encryption page.");
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const save = el("button", { type: "submit", class: "primary" }, "Set");
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Default encryption: "), mono(bucket.name)),
    choice("none", "None", "a write is sealed only when it asks to be."),
    choice("AES256", "Server key", "every write that asks for nothing is sealed under the server's key (SSE-S3)."),
    choice("aws:kms", "Named key", "every write that asks for nothing is sealed under the key named below (SSE-KMS)."),
    key.row,
    reason.row,
    el("div", { class: "actions" }, save, cancel),
    said,
  );
  cancel.addEventListener("click", close);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const picked = form.querySelector<HTMLInputElement>(`input[name="${id}-algorithm"]:checked`);
    if (picked === null) {
      fill(said, el("p", { class: "message bad", role: "alert" }, "Choose one."));
      return;
    }
    const name = key.input.value.trim();
    if (picked.value === "aws:kms" && !KEY_NAME.test(name)) {
      fill(said, el("p", { class: "message bad", role: "alert" }, "A key name is 1-64 lowercase letters, digits and hyphens."));
      key.input.focus();
      return;
    }
    const body = picked.value === "aws:kms" ? { algorithm: picked.value, key: name, reason: reason.input.value.trim() } : { algorithm: picked.value, reason: reason.input.value.trim() };
    const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket.name)}/encryption`, ignored, body);
    if (answer.ok) {
      announce(`Set the default encryption of ${bucket.name}.`);
      screen.redraw();
      return;
    }
    if (answer.status === 401) {
      screen.signIn();
      return;
    }
    fill(said, refusal(answer));
  });
  return form;
}
