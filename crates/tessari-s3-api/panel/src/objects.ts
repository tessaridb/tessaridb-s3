//! A bucket's keys a page at a time, in the server's byte order, rolled up at
//! `/` like a folder tree. Paging is by the server's cursor alone: there is no
//! total to show, because counting a large bucket is a scan. Selecting a key
//! shows its metadata in a side pane beside the list (the selection is in the
//! URL); the arrow keys move the selection and Escape closes the pane.

import { call, search } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { kindOf } from "./file-kind.ts";
import { moment, size } from "./format.ts";
import { icon } from "./icons.ts";
import { readListing } from "./models.ts";
import { describe } from "./object-pane.ts";
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
    last ? el("span", { "aria-current": "page" }, label) : el("a", { href: format({ kind: "objects", bucket, prefix: at, cursor: null, key: null }) }, label);
  steps.push(step(bucket, "", parts.length === 0));
  let walked = "";
  parts.forEach((part, index) => {
    walked += `${part}/`;
    steps.push(icon("chevron"), step(part, walked, index === parts.length - 1));
  });
  return el("nav", { class: "trail", "aria-label": "Prefix" }, ...steps);
}

export async function objects(screen: Screen, bucket: string, prefix: string, cursor: string | null, selected: string | null): Promise<void> {
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
  const here = (key: string | null): string => format({ kind: "objects", bucket, prefix, cursor, key });
  const links = new Map<string, HTMLAnchorElement>();
  const folders = page.prefixes.map((folder) =>
    row(el("a", { class: "name", href: format({ kind: "objects", bucket, prefix: folder, cursor: null, key: null }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", "", ""),
  );
  const files = page.objects.map((object) => {
    const kind = kindOf(object.contentType);
    const link = el("a", { class: "name", href: here(object.key), "data-key": object.key }, icon(kind.icon), mono(object.key.slice(prefix.length)));
    link.addEventListener("click", (event) => {
      // A plain click selects in place; a modified one opens the URL as the browser would.
      if (event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) {
        event.preventDefault();
        select(object.key);
      }
    });
    links.set(object.key, link);
    return row(
      link,
      el("span", { title: object.contentType ?? "No content type was stored" }, kind.label),
      el("span", { title: `${object.size.toLocaleString()} bytes` }, size(object.size)),
      moment(object.modified),
      mono(object.etag),
    );
  });
  const nothing =
    cursor !== null
      ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null }) }, "Back to the first page"))
      : prefix === ""
        ? empty("This bucket is empty.")
        : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket, prefix: "", cursor: null, key: null }) }, "Back to the top of the bucket"));
  const paging =
    cursor === null && page.next === null
      ? null
      : el(
    "p",
    { class: "paging" },
    cursor === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null }) }, "First page"),
    cursor !== null && page.next !== null ? " · " : null,
    page.next === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: page.next, key: null }) }, "Next page"),
  );
  const list = el(
    "section",
    { class: "card flush" },
    folders.length + files.length === 0 ? nothing : table(`Keys under ${prefix === "" ? "the bucket" : prefix}`, ["Name", "Type", "Size", "Last modified", "ETag"], [...folders, ...files]),
    paging,
  );
  const pane = el("aside", { class: "pane card", "aria-labelledby": "pane-title", hidden: "" });
  const split = el("div", { class: "split" }, list, pane);
  let shown: string | null = null;
  let turn = 0;
  const select = (key: string | null): void => {
    links.get(shown ?? "")?.closest("tr")?.removeAttribute("aria-current");
    shown = key;
    turn += 1;
    history.replaceState(null, "", here(key));
    if (key === null) {
      pane.hidden = true;
      split.classList.remove("open");
      fill(pane);
      return;
    }
    links.get(key)?.closest("tr")?.setAttribute("aria-current", "true");
    pane.hidden = false;
    split.classList.add("open");
    if (window.matchMedia("(max-width: 64rem)").matches) {
      pane.scrollIntoView({ block: "start" });
    }
    const mine = turn;
    void describe(screen, pane, bucket, key, close, () => mine === turn);
  };
  function close(): void {
    const was = shown;
    select(null);
    links.get(was ?? "")?.focus();
  }
  split.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && shown !== null) {
      event.preventDefault();
      close();
      return;
    }
    const target = event.target;
    if ((event.key !== "ArrowDown" && event.key !== "ArrowUp") || !(target instanceof HTMLAnchorElement) || !target.classList.contains("name")) {
      return;
    }
    const names = Array.from(list.querySelectorAll<HTMLAnchorElement>("a.name"));
    const next = names[names.indexOf(target) + (event.key === "ArrowDown" ? 1 : -1)];
    if (next === undefined) {
      return;
    }
    event.preventDefault();
    next.focus();
    const key = next.dataset["key"];
    if (key !== undefined) {
      select(key);
    }
  });
  fill(screen.main, head(title, trail(bucket, prefix)), split);
  if (selected !== null) {
    select(selected);
  }
}
