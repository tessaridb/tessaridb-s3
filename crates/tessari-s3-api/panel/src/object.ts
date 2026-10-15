//! One object: what the server holds for it, a recorded download, and a delete
//! that only goes through while the object is still the version shown here —
//! the ETag on screen is the condition, so a rewrite in between is refused.

import { call, path, search } from "./api.ts";
import { confirmation } from "./confirm.ts";
import { announce, el, field, fill, mono } from "./dom.ts";
import { icon } from "./icons.ts";
import { ignored, readDetail } from "./models.ts";
import { deleteRefusal } from "./object-delete.ts";
import { facts, pairs } from "./object-facts.ts";
import { format } from "./route.ts";
import { failed, head, loading, type Screen } from "./screen.ts";

const TITLE = "Object";

/** The prefix a key sits under, so the listing it came from can be shown again. */
const parent = (key: string): string => key.slice(0, key.lastIndexOf("/") + 1);

export async function object(screen: Screen, bucket: string, key: string): Promise<void> {
  loading(screen, TITLE, "the object");
  const where = `/buckets/${encodeURIComponent(bucket)}/object`;
  const answer = await call("GET", `${where}${search([["key", key]])}`, readDetail);
  if (!answer.ok) {
    failed(screen, TITLE, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const detail = answer.value;
  const back = format({ kind: "objects", bucket, prefix: parent(key), cursor: null, key, filter: "" });

  const reason = field("download-reason", "Reason (optional)", { maxlength: "500" }, "Downloads are recorded with your key id.");
  const download = el("a", { class: "button", download: "" }, icon("download"), "Download");
  const point = (): void => {
    const why = reason.input.value.trim();
    download.setAttribute("href", `${path(`${where}/content`)}${search([["key", key], ["reason", why === "" ? null : why]])}`);
  };
  reason.input.addEventListener("input", point);
  point();

  const remove = el("button", { type: "button", class: "danger" }, icon("trash"), "Delete object…");
  const asking = el("div", {});
  remove.addEventListener("click", () => {
    remove.hidden = true;
    fill(
      asking,
      confirmation(
        "delete-object",
        mono(`${bucket}/${key}`),
        `There is no versioning: the object is gone for every client. It is deleted only if it is still ETag ${detail.etag}.`,
        "Delete object",
        async (why) => {
          const done = await call("DELETE", `${where}${search([["key", key]])}`, ignored, { etag: detail.etag, reason: why });
          if (done.ok) {
            announce(`Deleted ${key} from ${bucket}.`);
            location.hash = back;
            return null;
          }
          if (done.status === 401) {
            screen.signIn();
            return null;
          }
          return deleteRefusal(done);
        },
        () => {
          fill(asking);
          remove.hidden = false;
          remove.focus();
        },
      ),
    );
  });

  const name = key.slice(key.lastIndexOf("/") + 1) || key;
  fill(
    screen.main,
    head(name, el("span", {}, el("a", { class: "back", href: back }, icon("back"), "Back to the listing"), " · ", mono(`${bucket}/${key}`))),
    el("section", { class: "card" }, facts(detail)),
    el("section", { class: "card" }, el("h2", {}, "Actions"), reason.row, el("div", { class: "actions" }, download, remove), asking),
    el("section", { class: "card flush" }, el("h2", {}, "Headers"), pairs("Stored headers", detail.headers)),
    el("section", { class: "card flush" }, el("h2", {}, "User metadata"), pairs("User metadata", detail.metadata)),
    el("section", { class: "card flush" }, el("h2", {}, "Checksums"), pairs("Stored checksums", detail.checksums)),
  );
}
