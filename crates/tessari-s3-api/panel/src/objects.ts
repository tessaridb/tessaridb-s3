//! A bucket's keys a page at a time, in the server's byte order, rolled up at
//! `/` like a folder tree. Paging is by the server's cursor alone: there is no
//! total to show, because counting a large bucket is a scan.

import { call, search } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { moment, size } from "./format.ts";
import { icon } from "./icons.ts";
import { readListing } from "./models.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

/** Keys per page. */
const PAGE = 100;

/** The trail from the bucket down to `prefix`, each step a link. */
function trail(bucket: string, prefix: string): HTMLElement {
  const steps: Node[] = [
    el("a", { href: format({ kind: "buckets" }) }, "Buckets"),
    icon("chevron"),
  ];
  const parts = prefix.split("/").filter((piece) => piece !== "");
  // Every step links back up; the last one is where the operator already is.
  const step = (label: string, at: string, last: boolean): Node =>
    last ? el("span", { "aria-current": "page" }, label) : el("a", { href: format({ kind: "objects", bucket, prefix: at, cursor: null }) }, label);
  steps.push(step(bucket, "", parts.length === 0));
  let walked = "";
  parts.forEach((part, index) => {
    walked += `${part}/`;
    steps.push(icon("chevron"), step(part, walked, index === parts.length - 1));
  });
  return el("nav", { class: "trail", "aria-label": "Prefix" }, ...steps);
}

export async function objects(screen: Screen, bucket: string, prefix: string, cursor: string | null): Promise<void> {
  const title = bucket;
  loading(screen, title, "keys");
  const answer = await call(
    "GET",
    `/buckets/${encodeURIComponent(bucket)}/objects${search([["prefix", prefix === "" ? null : prefix], ["delimiter", "/"], ["cursor", cursor], ["limit", PAGE]])}`,
    readListing,
  );
  if (!answer.ok) {
    failed(screen, title, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  const page = answer.value;
  const folders = page.prefixes.map((folder) =>
    row(el("a", { class: "name", href: format({ kind: "objects", bucket, prefix: folder, cursor: null }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", ""),
  );
  const files = page.objects.map((object) =>
    row(
      el("a", { class: "name", href: format({ kind: "object", bucket, key: object.key }) }, icon("file"), mono(object.key.slice(prefix.length))),
      el("span", { title: `${object.size.toLocaleString()} bytes` }, size(object.size)),
      moment(object.modified),
      mono(object.etag),
    ),
  );
  const nothing =
    cursor !== null
      ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null }) }, "Back to the first page"))
      : prefix === ""
        ? empty("This bucket is empty.")
        : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket, prefix: "", cursor: null }) }, "Back to the top of the bucket"));
  const paging =
    cursor === null && page.next === null
      ? null
      : el(
    "p",
    { class: "paging" },
    cursor === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null }) }, "First page"),
    cursor !== null && page.next !== null ? " · " : null,
    page.next === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: page.next }) }, "Next page"),
  );
  fill(
    screen.main,
    head(title, trail(bucket, prefix)),
    el(
      "section",
      { class: "card flush" },
      folders.length + files.length === 0 ? nothing : table(`Keys under ${prefix === "" ? "the bucket" : prefix}`, ["Name", "Size", "Last modified", "ETag"], [...folders, ...files]),
      paging,
    ),
  );
}
