//! Building the page. Everything the server hands over — bucket names, keys,
//! reasons, metadata — is S3 data that any client could have written, so it is
//! only ever placed with `textContent` or as an attribute value; nothing here
//! parses a string as markup.

type Child = Node | string | null | false;

/** A `tag` element with `attributes` and `children`; strings become text nodes. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attributes: Readonly<Record<string, string>> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const made = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    made.setAttribute(name, value);
  }
  for (const child of children) {
    if (child !== null && child !== false) {
      made.append(child);
    }
  }
  return made;
}

/** The element with this id; throws when the shell carries none, which is a build defect rather than a state. */
export function at(id: string): HTMLElement {
  const found = document.getElementById(id);
  if (found === null) {
    throw new Error(`the page has no element #${id}`);
  }
  return found;
}

/** Replaces everything in `parent` with `children`. */
export function fill(parent: Element, ...children: Child[]): void {
  parent.replaceChildren(...children.filter((child): child is Node | string => child !== null && child !== false));
}

/** Says `words` to a screen reader through the shell's polite live region, and shows them there. */
export function announce(words: string): void {
  at("status-line").textContent = words;
}

/** A labelled text control: the label stays visible; `hint` explains the format. */
export function field(
  id: string,
  label: string,
  attributes: Readonly<Record<string, string>> = {},
  hint?: string,
): { readonly row: HTMLElement; readonly input: HTMLInputElement } {
  const input = el("input", { id, name: id, type: "text", ...attributes });
  const described = hint === undefined ? null : el("p", { id: `${id}-hint`, class: "hint" }, hint);
  if (described !== null) {
    input.setAttribute("aria-describedby", described.id);
  }
  return { row: el("div", { class: "field" }, el("label", { for: id }, label), input, described), input };
}

/**
 * A few choices shown side by side, one picked: radios under one name, so the
 * arrow keys move between them and no popup opens. `chosen` reads the pick and
 * `choose` sets it.
 */
export function segmented(
  id: string,
  label: string,
  options: readonly string[],
): { readonly group: HTMLElement; readonly chosen: () => string; readonly choose: (value: string) => void } {
  const inputs = options.map((option, at) => {
    const input = el("input", { type: "radio", name: id, id: `${id}-${at}`, value: option });
    if (at === 0) {
      input.checked = true;
    }
    return input;
  });
  const group = el(
    "div",
    { class: "segmented", role: "radiogroup", "aria-label": label },
    ...inputs.flatMap((input) => [input, el("label", { for: input.id }, input.value)]),
  );
  const chosen = (): string => inputs.find((input) => input.checked)?.value ?? "";
  const choose = (value: string): void => {
    for (const input of inputs) {
      input.checked = input.value === value;
    }
  };
  return { group, chosen, choose };
}

/** A small table: `head` names the columns, `rows` are already built cells. */
export function table(caption: string, head: readonly string[], rows: readonly HTMLTableRowElement[]): HTMLElement {
  return el(
    "div",
    { class: "scroll" },
    el(
      "table",
      {},
      el("caption", { class: "sr-only" }, caption),
      el("thead", {}, el("tr", {}, ...head.map((name) => el("th", { scope: "col" }, name)))),
      el("tbody", {}, ...rows),
    ),
  );
}

/** A table row from cells, each either text or an element. */
export const row = (...cells: ReadonlyArray<Node | string>): HTMLTableRowElement =>
  el("tr", {}, ...cells.map((cell) => el("td", {}, cell)));

/** A monospaced value: keys, ETags, addresses — anything compared character by character. */
export const mono = (text: string): HTMLElement => el("code", {}, text);

/** Every element matching `selector`, as an array. */
export const all = (selector: string): HTMLElement[] => Array.from(document.querySelectorAll<HTMLElement>(selector));
