//! One run of a listing: its first page shown, and later pages appended from
//! the server's cursor as the end of the list comes into view or "Load more" is
//! pressed. There is no total to show, because counting a large bucket is a
//! scan. A new filter starts a new run and stops the old one.

import type { Answer } from "./api.ts";
import { el, table } from "./dom.ts";
import type { Listing } from "./models.ts";
import type { Screen } from "./screen.ts";

export type Run = {
  /** The table (or the empty state) and the paging line, to place in the list's card. */
  readonly nodes: readonly HTMLElement[];
  /** Starts appending as the end of the list comes into view; call once the nodes are on the page. */
  readonly start: () => void;
  /** Stops this run: a later answer for it is dropped. */
  readonly stop: () => void;
};

export function run(
  screen: Screen,
  first: Listing,
  rowsOf: (listing: Listing) => HTMLTableRowElement[],
  fetchPage: (after: string) => Promise<Answer<Listing>>,
  caption: string,
  nothing: HTMLElement,
  back: HTMLElement | null,
): Run {
  const rows = rowsOf(first);
  const body = rows.length === 0 ? nothing : table(caption, ["Name", "Type", "Size", "Last modified", "ETag", "Actions"], rows);
  let loaded = rows.length;
  let next = first.next;
  let busy = false;
  let stopped = false;
  const status = el("span", { role: "status" });
  const more = el("button", { type: "button" }, "Load more");
  const paging = el("p", { class: "paging" }, status, more, back);
  const settle = (): void => {
    more.hidden = next === null;
    const shown = loaded === 1 ? "1 key or folder shown" : `${loaded.toLocaleString()} keys and folders shown`;
    status.textContent = next !== null ? `${shown}.` : loaded === 1 ? `${shown}; there are no more.` : `All ${shown}.`;
  };
  /** Whether the end of the list is within a screen of being seen. */
  const near = (): boolean => paging.isConnected && paging.getBoundingClientRect().top < window.innerHeight * 2;
  const live = (): boolean => screen.live() && !stopped;
  const loadMore = async (): Promise<void> => {
    if (busy || next === null || !live()) {
      return;
    }
    // Busy rather than disabled: a disabled button drops the keyboard focus that pressed it.
    busy = true;
    more.setAttribute("aria-busy", "true");
    status.textContent = "Loading more keys…";
    const answer = await fetchPage(next);
    busy = false;
    more.removeAttribute("aria-busy");
    if (!live()) {
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
    const added = rowsOf(answer.value);
    body.querySelector("tbody")?.append(...added);
    loaded += added.length;
    next = answer.value.next;
    settle();
    if (near()) {
      void loadMore();
    }
  };
  more.addEventListener("click", () => void loadMore());
  const watch = new IntersectionObserver(
    (entries) => {
      if (!live() || next === null) {
        watch.disconnect();
      } else if (entries.some((entry) => entry.isIntersecting)) {
        void loadMore();
      }
    },
    { rootMargin: "0px 0px 100% 0px" },
  );
  settle();
  return {
    nodes: rows.length === 0 && next === null && back === null ? [body] : [body, paging],
    start: () => watch.observe(paging),
    stop: () => {
      stopped = true;
      watch.disconnect();
    },
  };
}
