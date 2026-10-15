//! Who acknowledges a bucket's object writes: the majority of the nodes keeping
//! its region's metadata (the default), or that region's majority alone. The
//! second is faster when the metadata is also kept in other regions, and it is
//! the one that can lose writes: an acknowledged write the region's nodes held
//! is lost if the metadata's leadership fails over to a node that did not
//! collect it. The server proves the region level before setting it and shows
//! the store's own refusal when there is no region to count. Operators only;
//! the reason goes into the action record.

import { call } from "./api.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { ignored, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";

/** The acknowledgement form for `bucket`, as a region `open` places under its row; `close` removes it. */
export function acknowledgementForm(screen: Screen, bucket: Bucket, close: () => void): HTMLElement {
  const id = `ack-${bucket.name}`;
  const choice = (value: Bucket["acknowledgement"], label: string, note: string): HTMLElement => {
    const input = el("input", { type: "radio", name: `${id}-level`, id: `${id}-${value}`, value });
    if (bucket.acknowledgement === value) {
      input.setAttribute("checked", "");
    }
    return el("div", { class: "choice" }, input, el("label", { for: `${id}-${value}` }, el("strong", {}, label), " — ", note));
  };
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  const save = el("button", { type: "submit", class: "primary" }, "Set");
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Write acknowledgement: "), mono(bucket.name)),
    choice("majority", "Majority", "a write is acknowledged once most of the nodes keeping this region's metadata hold it; it survives a failover."),
    choice("region", "Region majority", "acknowledged once most of this region's nodes hold it — faster, and a failover to a node outside the region can lose writes acknowledged this way."),
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
    const picked = form.querySelector<HTMLInputElement>(`input[name="${id}-level"]:checked`);
    if (picked === null) {
      fill(said, el("p", { class: "message bad", role: "alert" }, "Choose one."));
      return;
    }
    const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket.name)}/acknowledgement`, ignored, {
      level: picked.value,
      reason: reason.input.value.trim(),
    });
    if (answer.ok) {
      announce(`Set the write acknowledgement of ${bucket.name}.`);
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
