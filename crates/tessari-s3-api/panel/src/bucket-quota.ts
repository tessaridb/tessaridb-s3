//! A bucket's quota: the limits as the row shows them, and the form an operator
//! sets them with, inline under the row. Both limits are sent every time, so a
//! repeated save changes nothing; an empty field is no limit. The reason is
//! required and goes into the action record.

import { call } from "./api.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { amount, bytesOf, QUOTA_UNITS, size } from "./format.ts";
import { ignored, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";

type Unit = keyof typeof QUOTA_UNITS;

const isUnit = (value: string): value is Unit => Object.hasOwn(QUOTA_UNITS, value);

/** The row's "Limit" cell: what the bucket may hold, or that nothing limits it. */
export function limits(bucket: Bucket): string {
  const parts = [
    bucket.maxBytes === null ? null : size(bucket.maxBytes),
    bucket.maxObjects === null ? null : `${amount(bucket.maxObjects)} objects`,
  ].filter((part) => part !== null);
  return parts.length === 0 ? "—" : parts.join(" · ");
}

/** Marks `input` invalid for assistive technology — `aria-invalid` must say "true"; an empty value reads as false. */
function flag(input: HTMLInputElement, wrong: boolean): void {
  if (wrong) {
    input.setAttribute("aria-invalid", "true");
  } else {
    input.removeAttribute("aria-invalid");
  }
}

/** The largest unit `bytes` is a whole number of, for showing a limit back in the form. */
function unitOf(bytes: number): Unit {
  const units: Unit[] = ["TiB", "GiB", "MiB"];
  return units.find((unit) => bytes % QUOTA_UNITS[unit] === 0) ?? "MiB";
}

/** The quota form for `bucket`, as a region `open` places under its row; `close` removes it. */
export function quotaForm(screen: Screen, bucket: Bucket, close: () => void): HTMLElement {
  const id = `quota-${bucket.name}`;
  const unit = el("select", { id: `${id}-unit`, "aria-label": "Unit" }, ...Object.keys(QUOTA_UNITS).map((name) => el("option", { value: name }, name)));
  const bytes = field(`${id}-bytes`, "Size limit", { inputmode: "decimal", autocomplete: "off" }, "Empty: no size limit. Checked against the last measurement.");
  const objects = field(`${id}-objects`, "Object limit", { inputmode: "numeric", autocomplete: "off" }, "Empty: no object limit.");
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  if (bucket.maxBytes !== null) {
    unit.value = unitOf(bucket.maxBytes);
    bytes.input.value = String(bucket.maxBytes / QUOTA_UNITS[unitOf(bucket.maxBytes)]);
  }
  if (bucket.maxObjects !== null) {
    objects.input.value = String(bucket.maxObjects);
  }
  bytes.input.after(unit);
  const save = el("button", { type: "submit", class: "primary" }, "Set quota");
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Quota: "), mono(bucket.name)),
    el("p", { class: "muted" }, "Writes that would pass a limit are refused. A bucket can pass it by what is written before the next measurement."),
    bytes.row,
    objects.row,
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
    const maxBytes = isUnit(unit.value) ? bytesOf(bytes.input.value, unit.value) : undefined;
    const written = objects.input.value.trim();
    const maxObjects = written === "" ? null : /^\d+$/.test(written) ? Number(written) : undefined;
    const objectsWrong = maxObjects === undefined || (maxObjects !== null && !Number.isSafeInteger(maxObjects));
    flag(bytes.input, maxBytes === undefined);
    flag(objects.input, objectsWrong);
    if (maxBytes === undefined || maxObjects === undefined || objectsWrong) {
      fill(said, el("p", { class: "message bad", role: "alert" }, "Write each limit as a number, or leave it empty for none."));
      return;
    }
    const answer = await call("PUT", `/buckets/${encodeURIComponent(bucket.name)}/quota`, ignored, {
      max_bytes: maxBytes,
      max_objects: maxObjects,
      reason: reason.input.value.trim(),
    });
    if (answer.ok) {
      announce(`Set the quota of ${bucket.name}.`);
      screen.redraw();
      return;
    }
    if (answer.status === 401) {
      screen.signIn();
      return;
    }
    fill(said, refusal(answer));
  });
  queueMicrotask(() => bytes.input.focus());
  return form;
}
