//! Buckets: find one (the list is complete, so filtering it here hides nothing
//! the server sent), create one from the form the header's button reveals, and
//! delete an empty one with a reason.

import { call } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono, row, table } from "./dom.ts";
import { moment } from "./format.ts";
import { icon } from "./icons.ts";
import { ignored, readBuckets, type Bucket } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, refusal, type Screen } from "./screen.ts";

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

function bucketRow(screen: Screen, bucket: Bucket): HTMLTableRowElement {
  const remove = el("button", { type: "button", class: "quiet" }, icon("trash"), "Delete…");
  const line = row(
    el("a", { class: "name", href: format({ kind: "objects", bucket: bucket.name, prefix: "", cursor: null }) }, icon("buckets"), mono(bucket.name)),
    moment(bucket.created),
    mono(bucket.region),
    remove,
  );
  remove.addEventListener("click", () => {
    const cell = el("td", { colspan: "4" });
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

export async function buckets(screen: Screen): Promise<void> {
  loading(screen, TITLE, "buckets");
  const answer = await call("GET", "/buckets", readBuckets);
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
          : table("Buckets", ["Name", "Created", "Region", "Actions"], shown.map((bucket) => bucketRow(screen, bucket))),
    );
  };
  filter.input.addEventListener("input", draw);
  draw();
  fill(
    screen.main,
    head(TITLE, `${all.length.toLocaleString()} on this cluster`, opener),
    form,
    el("section", { class: "card flush" }, all.length === 0 ? null : el("div", { class: "toolbar" }, filter.row), listed),
  );
}
