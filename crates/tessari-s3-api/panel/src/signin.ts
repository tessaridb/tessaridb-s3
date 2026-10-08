//! Signing in with the node's root access key. The secret goes to the server
//! once; what the browser keeps is an HttpOnly session cookie it cannot read,
//! and the fields are cleared as soon as the answer arrives.

import { call } from "./api.ts";
import { el, field, fill } from "./dom.ts";
import { ignored } from "./models.ts";
import { heading, refusal } from "./screen.ts";

/** Draws the sign-in form into `main`; `done` runs once a session exists. */
export function signIn(main: HTMLElement, done: () => void): void {
  const key = field("access-key", "Access key ID", { autocomplete: "username", spellcheck: "false", required: "" });
  const secret = field("secret-key", "Secret access key", { type: "password", autocomplete: "current-password", required: "" });
  const submit = el("button", { type: "submit", class: "primary" }, "Sign in");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "card", novalidate: "" },
    el("img", { class: "logo", src: "/favicon.svg", alt: "", width: "48", height: "48" }),
    heading("Sign in to TessariDB S3"),
    el("p", { class: "muted" }, "Use the root access key this node was started with. The session lasts one hour."),
    key.row,
    secret.row,
    submit,
    said,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    submit.disabled = true;
    const answer = await call("POST", "/session", ignored, {
      access_key_id: key.input.value.trim(),
      secret_access_key: secret.input.value,
    });
    secret.input.value = "";
    submit.disabled = false;
    if (answer.ok) {
      key.input.value = "";
      done();
      return;
    }
    const failure =
      answer.status === 401 ? { ...answer, message: "That access key and secret do not match this node's root credential." } : answer;
    fill(said, refusal(failure));
    secret.input.focus();
  });
  fill(main, el("div", { class: "gate" }, form));
  key.input.focus();
}
