//! A bucket's keys in the server's byte order, rolled up at `/` like a folder
//! tree. Pages are appended as the operator scrolls (or asks with "Load more"),
//! each one fetched from the server's cursor: there is no total to show,
//! because counting a large bucket is a scan. Selecting a key
//! shows its metadata in a side pane beside the list (the selection is in the
//! URL); the arrow keys move the selection and Escape closes the pane.

import { call, search } from "./api.ts";
import { el, fill, mono, row, table } from "./dom.ts";
import { kindOf } from "./file-kind.ts";
import { moment, size } from "./format.ts";
import { icon } from "./icons.ts";
import { type Listing, type ObjectRow, readListing } from "./models.ts";
import { objectActions } from "./object-actions.ts";
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
  const fetchPage = (after: string | null) =>
    call(
      "GET",
      `/buckets/${encodeURIComponent(bucket)}/objects${search([["prefix", prefix === "" ? null : prefix], ["delimiter", "/"], ["cursor", after], ["limit", PAGE]])}`,
      readListing,
    );
  const answer = await fetchPage(cursor);
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
  const folderRow = (folder: string): HTMLTableRowElement =>
    row(el("a", { class: "name", href: format({ kind: "objects", bucket, prefix: folder, cursor: null, key: null }) }, icon("folder"), mono(folder.slice(prefix.length))), "Folder", "", "", "", "");
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
  const first = rowsOf(page);
  const nothing =
    cursor !== null
      ? empty("No more keys on this page.", el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null }) }, "Back to the first page"))
      : prefix === ""
        ? empty("This bucket is empty.")
        : empty("Nothing under this prefix.", el("a", { href: format({ kind: "objects", bucket, prefix: "", cursor: null, key: null }) }, "Back to the top of the bucket"));
  let loaded = first.length;
  let next = page.next;
  let busy = false;
  const status = el("span", { role: "status" });
  const more = el("button", { type: "button" }, "Load more");
  const paging = el(
    "p",
    { class: "paging" },
    status,
    more,
    cursor === null ? null : el("a", { href: format({ kind: "objects", bucket, prefix, cursor: null, key: null }) }, "Back to the first page"),
  );
  const settle = (): void => {
    more.hidden = next === null;
    status.textContent = next === null ? `All ${loaded.toLocaleString()} keys and folders shown.` : `${loaded.toLocaleString()} keys and folders shown.`;
  };
  const list = el(
    "section",
    { class: "card flush" },
    first.length === 0 ? nothing : table(`Keys under ${prefix === "" ? "the bucket" : prefix}`, ["Name", "Type", "Size", "Last modified", "ETag", "Actions"], first),
    first.length === 0 && next === null ? null : paging,
  );
  /** Whether the end of the list is within a screen of being seen. */
  const near = (): boolean => paging.isConnected && paging.getBoundingClientRect().top < window.innerHeight * 2;
  const loadMore = async (): Promise<void> => {
    if (busy || next === null) {
      return;
    }
    // Busy rather than disabled: a disabled button drops the keyboard focus that pressed it.
    busy = true;
    more.setAttribute("aria-busy", "true");
    status.textContent = "Loading more keys…";
    const answer = await fetchPage(next);
    busy = false;
    more.removeAttribute("aria-busy");
    if (!screen.live()) {
      return;
    }
    if (!answer.ok) {
      if (answer.status === 401) {
        screen.signIn();
        return;
      }
      status.textContent = `${answer.message} Use "Load more" to try again.`;
      return;
    }
    const rows = rowsOf(answer.value);
    list.querySelector("tbody")?.append(...rows);
    loaded += rows.length;
    next = answer.value.next;
    settle();
    if (near()) {
      void loadMore();
    }
  };
  more.addEventListener("click", () => void loadMore());
  const watch = new IntersectionObserver(
    (entries) => {
      if (!screen.live() || next === null) {
        watch.disconnect();
      } else if (entries.some((entry) => entry.isIntersecting)) {
        void loadMore();
      }
    },
    { rootMargin: "0px 0px 100% 0px" },
  );
  settle();
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
  watch.observe(paging);
  if (selected !== null) {
    select(selected);
  }
}
