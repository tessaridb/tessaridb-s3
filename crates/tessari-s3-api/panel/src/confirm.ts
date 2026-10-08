//! The confirmation for an irreversible change to one thing: it names the thing,
//! says what cannot be undone, and will not proceed without a reason — the
//! reason goes into the action record beside the operator's key id.

import { el, fill } from "./dom.ts";
import { refusal, type Failure } from "./screen.ts";

/** The longest reason the server accepts. */
const REASON_MAX = 500;

/**
 * An inline confirmation region. `act(reason)` performs the change and answers
 * null on success or the failure to show; `cancel` removes the region.
 */
export function confirmation(
  id: string,
  what: Node,
  consequence: string,
  button: string,
  act: (reason: string) => Promise<Failure | null>,
  cancel: () => void,
): HTMLElement {
  const reason = el("textarea", { id: `${id}-reason`, rows: "2", maxlength: String(REASON_MAX), required: "", "aria-describedby": `${id}-reason-hint` });
  const hint = el("p", { id: `${id}-reason-hint`, class: "hint" }, "Required. Recorded with your key id in the action record.");
  const go = el("button", { type: "submit", class: "danger" }, button);
  const back = el("button", { type: "button" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", "aria-labelledby": `${id}-title`, novalidate: "" },
    el("p", { id: `${id}-title` }, el("strong", {}, `${button}: `), what),
    el("p", { class: "muted" }, consequence),
    el("div", { class: "field" }, el("label", { for: reason.id }, "Reason"), reason, hint),
    el("div", { class: "actions" }, go, back),
    said,
  );
  back.addEventListener("click", cancel);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      cancel();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const why = reason.value.trim();
    if (why === "") {
      reason.setAttribute("aria-invalid", "true");
      fill(said, el("p", { class: "message bad", role: "alert" }, "Say why — a reason is required for this action."));
      reason.focus();
      return;
    }
    reason.removeAttribute("aria-invalid");
    go.disabled = true;
    const failure = await act(why);
    go.disabled = false;
    if (failure !== null) {
      fill(said, refusal(failure));
    }
  });
  queueMicrotask(() => reason.focus());
  return form;
}
