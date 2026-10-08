//! Issuing a one-key upload credential for a bucket, inline under its row: the
//! key it may upload, how long it lasts, and a reason that is recorded. The
//! credential uploads that one key until it expires, and only while the issuer
//! may still write the bucket. Its secret is shown once, here.

import { call } from "./api.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { moment } from "./format.ts";
import { readUploadKey, type Bucket } from "./models.ts";
import { refusal, type Screen } from "./screen.ts";
import { shownOnce } from "./user-actions.ts";

/** How long a credential may last, in seconds: the server accepts 60 to 604800. */
const LIFETIMES = { "1 hour": 3_600, "1 day": 86_400, "7 days": 604_800 } as const;

type Lifetime = keyof typeof LIFETIMES;

const isLifetime = (value: string): value is Lifetime => Object.hasOwn(LIFETIMES, value);

/** The upload-key form for `bucket`, as a region `open` places under its row; `close` removes it. */
export function uploadKeyForm(screen: Screen, bucket: Bucket, close: () => void): HTMLElement {
  const id = `upload-${bucket.name}`;
  const key = field(`${id}-key`, "Object key", { autocomplete: "off", required: "" }, "The one key it may upload, exactly as written.");
  const lasts = el("select", { id: `${id}-lasts`, "aria-label": "Lasts" }, ...Object.keys(LIFETIMES).map((name) => el("option", { value: name }, name)));
  const reason = field(`${id}-reason`, "Reason", { maxlength: "500", required: "" }, "Required. Recorded with your key id.");
  key.input.after(lasts);
  const issue = el("button", { type: "submit", class: "primary" }, "Issue upload key");
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const said = el("div", { "aria-live": "polite" });
  const region = el("div", {});
  const form = el(
    "form",
    { class: "confirm", novalidate: "", "aria-labelledby": `${id}-title` },
    el("p", { id: `${id}-title` }, el("strong", {}, "Upload key: "), mono(bucket.name)),
    el("p", { class: "muted" }, "It uploads one key — a single PUT or a multipart upload — and nothing else, until it expires or you lose write access here."),
    key.row,
    reason.row,
    el("div", { class: "actions" }, issue, cancel),
    said,
  );
  fill(region, form);
  cancel.addEventListener("click", close);
  form.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      close();
    }
  });
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const wanted = key.input.value;
    if (wanted === "" || !isLifetime(lasts.value)) {
      key.input.setAttribute("aria-invalid", "true");
      fill(said, el("p", { class: "message bad", role: "alert" }, "Name the key it may upload."));
      return;
    }
    key.input.removeAttribute("aria-invalid");
    const answer = await call("POST", `/buckets/${encodeURIComponent(bucket.name)}/upload-keys`, readUploadKey, {
      key: wanted,
      expires_in: LIFETIMES[lasts.value],
      reason: reason.input.value.trim(),
    });
    if (answer.ok) {
      announce(`Issued an upload key for ${bucket.name}/${wanted}. Store its secret now.`);
      fill(
        region,
        el("p", {}, "Uploads ", mono(`${bucket.name}/${wanted}`), " until ", moment(answer.value.expires), "."),
        shownOnce(answer.value, close),
      );
      return;
    }
    if (answer.status === 401) {
      screen.signIn();
      return;
    }
    fill(said, refusal(answer));
  });
  queueMicrotask(() => key.input.focus());
  return region;
}
