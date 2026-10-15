//! An object's stored facts as the console shows them — on the object's own
//! page and in the listing's side pane. Everything here comes from the object's
//! record; nothing reads the object's bytes.

import { el, mono, row, table } from "./dom.ts";
import { kindOf } from "./file-kind.ts";
import { moment, size } from "./format.ts";
import type { Detail } from "./models.ts";

/** Name–value pairs as a table, or a plain "None." */
export function pairs(caption: string, values: Readonly<Record<string, string>>): HTMLElement {
  const entries = Object.entries(values);
  return entries.length === 0
    ? el("div", { class: "empty" }, el("p", {}, "None."))
    : table(caption, ["Name", "Value"], entries.map(([name, value]) => row(mono(name), mono(value))));
}

/** Type, size, ETag, age and how it was uploaded. */
export function facts(detail: Detail): HTMLElement {
  const type = detail.headers["content-type"] ?? null;
  return el(
    "dl",
    { class: "facts" },
    el("dt", {}, "Type"),
    el("dd", {}, type === null ? "None stored" : el("span", {}, `${kindOf(type).label} · `, mono(type))),
    el("dt", {}, "Size"),
    el("dd", {}, `${size(detail.size)} (${detail.size.toLocaleString()} bytes)`),
    el("dt", {}, "ETag"),
    el("dd", {}, mono(detail.etag)),
    el("dt", {}, "Last modified"),
    el("dd", {}, moment(detail.modified)),
    el("dt", {}, "Upload"),
    el("dd", {}, detail.parts === null ? "Single request" : `Multipart, ${detail.parts} parts`),
  );
}
