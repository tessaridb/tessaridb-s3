//! Buckets: find one (the list is complete, so filtering it here hides nothing
//! the server sent), see how many objects and bytes each holds as last measured,
//! create one from the form the header's button reveals, delete an empty one
//! with a reason, and — for operators — set its quota.

import { call, type Answer } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { amount, moment, size } from "./format.ts";
import { icon } from "./icons.ts";
import { ignored, readBuckets, readUsage, type Bucket, type BucketUsage, type Usage } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, refusal, type Screen } from "./screen.ts";
import { limits, quotaForm } from "./bucket-quota.ts";
import { uploadKeyForm } from "./bucket-upload-key.ts";

const TITLE = "Buckets";

/** The create form, hidden until `opener` is pressed; closing it returns focus to `opener`. */
function createForm(screen: Screen, opener: HTMLButtonElement): HTMLElement {
  const name = field("new-bucket", "Bucket name", { spellcheck: "false", autocomplete: "off", required: "" }, "3-63 lowercase letters, digits, dots and hyphens.");
  const reason = field("new-bucket-reason", "Reason (optional)", { maxlength: "500" });
  const said = el("div", { "aria-live": "polite" });
  const cancel = el("button", { type: "button", class: "quiet" }, "Cancel");
  const form = el(
    "form",
    { class: "card", novalidate: "", hidden: "", "aria-labelledby": "new-bucket-title" },
    el("h2", { id: "new-bucket-title" }, "New bucket"),
    name.row,
    reason.row,
    el("div", { class: "actions" }, el("button", { type: "submit", class: "primary" }, "Create bucket"), cancel),
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
    const why = reason.input.value.trim();
    const answer = await call("POST", "/buckets", ignored, why === "" ? { name: wanted } : { name: wanted, reason: why });
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      name.input.setAttribute("aria-invalid", "true");
      fill(said, refusal(answer));
      return;
    }
    announce(`Created bucket ${wanted}.`);
    screen.redraw();
  });
  return form;
}

/** A bucket's figures as last measured: none when nothing was measured since it was created, zero when it was
 * measured and held nothing. ISO instants in one format compare as text. */
function figures(usage: Answer<Usage>, bucket: Bucket): BucketUsage | null {
  if (!usage.ok || usage.value.taken === null || usage.value.taken < bucket.created) {
    return null;
  }
  return usage.value.buckets.find((entry) => entry.bucket === bucket.name) ?? { bucket: bucket.name, objects: 0, bytes: 0, inline_bytes: 0, raw_bytes: 0 };
}

const numeric = (text: string): HTMLElement => el("span", { class: "numeric" }, text);

function bucketRow(screen: Screen, bucket: Bucket, usage: Answer<Usage>): HTMLTableRowElement {
  const remove = el("button", { type: "button", class: "quiet" }, icon("trash"), "Delete…");
  // Offered to keys that may operate; the server refuses everyone else regardless.
  const quota = screen.may?.operate === true ? el("button", { type: "button", class: "quiet" }, "Quota…") : null;
  // Offered to users; the server issues one only to a user who may write this bucket.
  const upload = screen.may?.issue_upload_keys === true ? el("button", { type: "button", class: "quiet" }, "Upload key…") : null;
  const held = figures(usage, bucket);
  const line = row(
    el("a", { class: "name", href: format({ kind: "objects", bucket: bucket.name, prefix: "", cursor: null }) }, icon("buckets"), mono(bucket.name)),
    numeric(held === null ? "—" : amount(held.objects)),
    numeric(held === null ? "—" : size(held.bytes)),
    numeric(held === null ? "—" : size(held.raw_bytes)),
    numeric(limits(bucket)),
    moment(bucket.created),
    mono(bucket.region),
    el("div", { class: "actions" }, upload, quota, remove),
  );
  quota?.addEventListener("click", () => {
    const cell = el("td", { colspan: "8" });
    const region = el("tr", { class: "asking" }, cell);
    cell.append(
      quotaForm(screen, bucket, () => {
        region.remove();
        quota.focus();
      }),
    );
    line.after(region);
  });
  upload?.addEventListener("click", () => {
    const cell = el("td", { colspan: "8" });
    const region = el("tr", { class: "asking" }, cell);
    cell.append(
      uploadKeyForm(screen, bucket, () => {
        region.remove();
        upload.focus();
      }),
    );
    line.after(region);
  });
  remove.addEventListener("click", () => {
    const cell = el("td", { colspan: "8" });
    const ask = el("tr", { class: "asking" }, cell);
    const close = (): void => {
      ask.remove();
      remove.focus();
    };
    cell.append(
      confirmation(
        `delete-${bucket.name}`,
        mono(bucket.name),
        "Only an empty bucket can be deleted, and the name becomes free for anyone to take.",
        "Delete bucket",
        async (reason) => {
          const answer = await call("DELETE", `/buckets/${encodeURIComponent(bucket.name)}`, ignored, { reason });
          if (answer.ok) {
            announce(`Deleted bucket ${bucket.name}.`);
            screen.redraw();
            return null;
          }
          if (answer.status === 401) {
            screen.signIn();
            return null;
          }
          return answer;
        },
        close,
      ),
    );
    line.after(ask);
  });
  return line;
}

/** The header's line: how many buckets, and how old the sizes are. */
function measuredLine(count: number, usage: Answer<Usage>): string {
  const listed = `${count.toLocaleString()} on this cluster`;
  if (!usage.ok) {
    return `${listed} · sizes could not be read`;
  }
  return usage.value.taken === null ? `${listed} · sizes not measured yet` : `${listed} · sizes measured ${moment(usage.value.taken)}`;
}

export async function buckets(screen: Screen): Promise<void> {
  loading(screen, TITLE, "buckets");
  const [answer, usage] = await Promise.all([call("GET", "/buckets", readBuckets), call("GET", "/usage", readUsage)]);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const all = answer.value;
  const opener = el("button", { type: "button", class: "primary", "aria-expanded": "false", "aria-controls": "new-bucket-form" }, icon("plus"), "New bucket");
  const form = createForm(screen, opener);
  form.id = "new-bucket-form";
  const filter = field("bucket-filter", "Filter by name", { type: "search", spellcheck: "false", autocomplete: "off" });
  const listed = el("div", {});
  const draw = (): void => {
    const wanted = filter.input.value.trim();
    const shown = all.filter((bucket) => bucket.name.includes(wanted));
    fill(
      listed,
      all.length === 0
        ? empty("No buckets yet. Create one with “New bucket”, or with any S3 client.")
        : shown.length === 0
          ? empty(`No bucket name contains “${wanted}”.`)
          : table("Buckets", ["Name", "Objects", "Size", "On drives", "Limit", "Created", "Region", "Actions"], shown.map((bucket) => bucketRow(screen, bucket, usage))),
    );
  };
  filter.input.addEventListener("input", draw);
  draw();
  fill(
    screen.main,
    head(TITLE, measuredLine(all.length, usage), opener),
    form,
    el("section", { class: "card flush" }, all.length === 0 ? null : el("div", { class: "toolbar" }, filter.row), listed),
  );
}
