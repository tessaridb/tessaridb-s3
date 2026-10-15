//! Signing in — through the organisation's identity provider when this node offers one, or with an access key: the
//! node's root key or a key issued to a user. The secret goes to the server once; what the browser keeps is an
//! HttpOnly session cookie it cannot read, and the fields are cleared as soon as the answer arrives. Beside a
//! provider the root key is the emergency path: the server asks for a reason, and the form then shows that field.

import { call } from "./api.ts";
import { el, field, fill } from "./dom.ts";
import { ignored } from "./models.ts";
import { heading, refusal } from "./screen.ts";
import { PROVIDER_SIGN_IN, readMethods } from "./signin-model.ts";

/** The provider's sign-in, offered above the key form when the node has one; nothing when it has none or did not say. */
async function providerChoice(): Promise<HTMLElement | null> {
  const methods = await call("GET", "/auth/methods", readMethods);
  if (!methods.ok || !methods.value.provider) {
    return null;
  }
  return el(
    "div",
    {},
    el("a", { class: "button primary provider", href: PROVIDER_SIGN_IN }, "Sign in with your organisation"),
    el("p", { class: "muted" }, "or with an access key"),
  );
}

/** Draws the sign-in form into `main`; `done` runs once a session exists. */
export function signIn(main: HTMLElement, done: () => void): void {
  const key = field("access-key", "Access key ID", { autocomplete: "username", spellcheck: "false", required: "" });
  const secret = field("secret-key", "Secret access key", { type: "password", autocomplete: "current-password", required: "" });
  const reason = field("reason", "Reason", { autocomplete: "off" }, "Why the root key, now: it is recorded and announced.");
  reason.row.hidden = true;
  const submit = el("button", { type: "submit", class: "primary" }, "Sign in");
  const said = el("div", { "aria-live": "polite" });
  const choice = el("div", {});
  const intro = el("p", { class: "muted" }, "Use your access key: one issued to your user, or the root key this node was started with. The session lasts one hour.");
  const form = el(
    "form",
    { class: "card", novalidate: "" },
    el("img", { class: "logo", src: "/favicon.svg", alt: "", width: "48", height: "48" }),
    heading("Sign in to TessariDB S3"),
    intro,
    choice,
    key.row,
    secret.row,
    reason.row,
    submit,
    said,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    submit.disabled = true;
    const why = reason.input.value.trim();
    const answer = await call("POST", "/session", ignored, {
      access_key_id: key.input.value.trim(),
      secret_access_key: secret.input.value,
      ...(why === "" ? {} : { reason: why }),
    });
    secret.input.value = "";
    submit.disabled = false;
    if (answer.ok) {
      key.input.value = "";
      reason.input.value = "";
      done();
      return;
    }
    if (answer.code === "reason_required") {
      reason.row.hidden = false;
      fill(said, refusal({ ...answer, message: "This node signs people in through its identity provider; the root key is its emergency path. Say why, then sign in again: the session lasts 15 minutes and is recorded and announced." }));
      reason.input.focus();
      return;
    }
    const failure =
      answer.status === 401 ? { ...answer, message: "That access key and secret do not match this node's root credential." } : answer;
    fill(said, refusal(failure));
    secret.input.focus();
  });
  fill(main, el("div", { class: "gate" }, form));
  key.input.focus();
  void providerChoice().then((offered) => {
    if (offered !== null) {
      // One primary action: the organisation's sign-in; a key is the other way in.
      fill(intro, "Sign in through your organisation. An access key also works: one issued to your user, or this node's root key in an emergency.");
      submit.classList.remove("primary");
      fill(choice, offered);
    }
  });
}
