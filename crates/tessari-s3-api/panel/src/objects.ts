//! A bucket's keys in the server's byte order, rolled up at `/` like a folder
//! tree, paged by the server's cursor as the list is scrolled. A filter keeps
//! the names in the open folder that start with some text — it is a longer
//! prefix, so the store lists only those. Selecting a key shows its metadata in
//! a side pane beside the list (the selection is in the URL); the arrow keys
//! move the selection and Escape closes the pane.

import { call, search } from "./api.ts";
import { el, field, fill, mono, row } from "./dom.ts";
import { kindOf } from "./file-kind.ts";
import { moment, size } from "./format.ts";
import { icon } from "./icons.ts";
import { type Listing, type ObjectRow, readListing } from "./models.ts";
import { objectActions } from "./object-actions.ts";
import { run, type Run } from "./object-pages.ts";
import { describe } from "./object-pane.ts";
import { format } from "./route.ts";
import { empty, failed, head, loading, type Screen } from "./screen.ts";

/** Keys per page. */
const PAGE = 100;

/** How long typing pauses before the filter is asked of the server. */
const FILTER_PAUSE_MS = 300;

/** The trail from the bucket down to `prefix`, each step a link. */
function trail(bucket: string, prefix: string): HTMLElement {
  const steps: Node[] = [
    el("a", { href: format({ kind: "buckets" }) }, "Buckets"),
    icon("chevron"),
  ];
  const parts = prefix.split("/").filter((piece) => piece !== "");
  // Every step links back up; the last one is where the operator already is.
  const step = (label: string, at: string, last: boolean): Node =>
    last ? el("span", { "aria-current": "page" }, label) : el("a", { href: format({ kind: "objects", bucket, prefix: at, cursor: null, key: null, filter: "" }) }, label);
  steps.push(step(bucket, "", parts.length === 0));
  let walked = "";
  parts.forEach((part, index) => {
    walked += `${part}/`;
    steps.push(icon("chevron"), step(part, walked, index === parts.length - 1));
  });
  return el("nav", { class: "trail", "aria-label": "Prefix" }, ...steps);
}

export async function objects(screen: Screen, bucket: string, prefix: string, cursor: string | null, selected: string | null, filter: string): Promise<void> {
  const title = bucket;
  loading(screen, title, "keys");
  const fetchPage = (named: string, after: string | null) =>
    call(
      "GET",
      `/buckets/${encodeURIComponent(bucket)}/objects${search([["prefix", `${prefix}${named}` === "" ? null : `${prefix}${named}`], ["delimiter", "/"], ["cursor", after], ["limit", PAGE]])}`,
      readListing,
    );
  const answer = await fetchPage(filter, cursor);
  if (!answer.ok) {
    failed(screen, title, answer);
    return;
  }
  if (!screen.live()) {
    return;
  }
  let named = filter;
  // The cursor a shared URL started from applies to the first run only.
  let from = cursor;
  const here = (key: string | null): string => format({ kind: "objects", bucket, prefix, cursor: from, key, filter: named });
  const links = new Map<string, HTMLAnchorElement>();
  const folderRow = (folder: string): HTMLTableRowElement =>
    row(el("a", { class: "name", href: format({ kind: "objects", bucket, prefix: folder, cursor: null, key: null, filter: "" }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", "", "", "");
  const fileRow = (object: ObjectRow): HTMLTableRowElement => {
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
    const buttons = el("div", { class: "actions" });
    const line = row(
      link,
      el("span", { title: object.contentType ?? "No content type was stored" }, kind.label),
      el("span", { title: `${object.size.toLocaleString()} bytes` }, size(object.size)),
      moment(object.modified),
      // The full ETag is in the details; eight hex digits tell two versions apart at a glance.
      el("span", { title: object.etag }, mono(object.etag.length > 12 ? `${object.etag.slice(0, 9)}…` : object.etag)),
      buttons,
    );
    const forget = (): void => {
      links.delete(object.key);
      if (shown === object.key) {
        select(null);
      }
    };
    buttons.append(...objectActions(screen, bucket, object, line, () => select(object.key), forget));
    return line;
  };
  const rowsOf = (listing: Listing): HTMLTableRowElement[] => [...listing.prefixes.map(folderRow), ...listing.objects.map(fileRow)];
  const nothing = (): HTMLElement =>
    named !== ""
      ? empty(`No name in this folder starts with “${named}”.`)
      : from !== null
        ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null, filter: "" }) }, "Back to the first page"))
        : prefix === ""
          ? empty("This bucket is empty.")
          : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket, prefix: "", cursor: null, key: null, filter: "" }) }, "Back to the top of the bucket"));
  const box = field("name-filter", "Filter by name", { type: "search", value: filter, autocomplete: "off", spellcheck: "false" }, "Names in this folder starting with this text; the server lists only those.");
  const toolbar = el("div", { class: "toolbar" }, box.row);
  const list = el("section", { class: "card flush" }, toolbar);
  let current: Run | null = null;
  /** Shows a run of `listing` below the filter, which keeps its focus and caret. */
  const show = (listing: Listing): void => {
    current?.stop();
    links.clear();
    const caption = `Keys under ${prefix === "" ? "the bucket" : prefix}${named === "" ? "" : ` starting “${named}”`}`;
    const back = from === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null, filter: named }) }, "Back to the first page");
    current = run(screen, listing, rowsOf, (after) => fetchPage(named, after), caption, nothing(), back);
    while (toolbar.nextSibling !== null) {
      toolbar.nextSibling.remove();
    }
    list.append(...current.nodes);
    current.start();
    links.get(shown ?? "")?.closest("tr")?.setAttribute("aria-current", "true");
  };
  let typing: number | undefined;
  let asked = 0;
  box.input.addEventListener("input", () => {
    window.clearTimeout(typing);
    typing = window.setTimeout(() => {
      void (async () => {
        if (box.input.value === named) {
          return;
        }
        named = box.input.value;
        from = null;
        asked += 1;
        const mine = asked;
        history.replaceState(null, "", here(shown));
        const answered = await fetchPage(named, null);
        if (!screen.live() || mine !== asked) {
          return;
        }
        if (answered.ok) {
          show(answered.value);
        } else if (answered.status === 401) {
          screen.signIn();
        } else {
          current?.stop();
          while (toolbar.nextSibling !== null) {
            toolbar.nextSibling.remove();
          }
          list.append(el("p", { class: "paging", role: "alert" }, answered.message));
        }
      })();
    }, FILTER_PAUSE_MS);
  });
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
    // Escape in the filter clears the filter, as a search field does; anywhere else it closes the pane.
    if (event.key === "Escape" && shown !== null && event.target !== box.input) {
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
  show(answer.value);
  fill(screen.main, head(title, trail(bucket, prefix)), split);
  if (selected !== null) {
    select(selected);
  }
}
