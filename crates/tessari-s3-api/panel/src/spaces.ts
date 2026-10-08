//! Spaces: the tenants buckets and users belong to. Only operators reach this
//! view; anyone else is told their key does not allow it. A new space takes a
//! reason, which goes into the action record.

import { call } from "./api.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { icon } from "./icons.ts";
import { ignored, readSpaces } from "./models.ts";
import { empty, failed, head, loading, refusal, type Screen } from "./screen.ts";

const TITLE = "Spaces";

function createForm(screen: Screen, opener: HTMLButtonElement): HTMLElement {
  const name = field("new-space", "Space name", { spellcheck: "false", autocomplete: "off", required: "" }, "1-63 lowercase letters, digits and inner hyphens.");
  const reason = field("new-space-reason", "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const said = el("div", { "aria-live": "polite" });
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const form = el(
    "form",
    { id: "new-space-form", class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-space-title" },
    el("h2", { id: "new-space-title" }, "New space"),
    name.row,
    reason.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create space"), cancel),
    said,
  );
  const close = (): void => {
    form.hidden = true;
    opener.setAttribute("aria-expanded", "false");
    opener.focus();
  };
  opener.addEventListener("click", () => {
    form.hidden = false;
    opener.setAttribute("aria-expanded", "true");
    name.input.focus();
  });
  cancel.addEventListener("click", close);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const wanted = name.input.value.trim();
    const answer = await call("POST", "/spaces", ignored, { name: wanted, reason: reason.input.value.trim() });
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      fill(said, refusal(answer));
      return;
    }
    announce(`Created space ${wanted}.`);
    screen.redraw();
  });
  return form;
}

export async function spaces(screen: Screen): Promise<void> {
  loading(screen, TITLE, "spaces");
  const answer = await call("GET", "/spaces", readSpaces);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-space-form" }, icon("plus"), "New space");
  fill(
    screen.main,
    head(TITLE, `${answer.value.length.toLocaleString()} on this store`, opener),
    createForm(screen, opener),
    el(
      "section",
      { class: "card flush" },
      answer.value.length === 0
        ? empty("No spaces yet.")
        : table("Spaces", ["Name", "Created"], answer.value.map((space) => row(mono(space.name), moment(space.created)))),
    ),
  );
}
