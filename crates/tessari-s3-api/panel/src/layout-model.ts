//! A layout change as the console sees it: the staged layout (what applying it
//! would make the next version) and the sets an operator types to stage one —
//! one erasure set per line, its node ids separated by spaces or commas.

import { count, layout, record, text } from "./models.ts";
import type { Layout } from "./models.ts";

/** A staged layout: `id` names this staging, `layout` is version `base` + 1. */
export type Staged = { readonly id: string; readonly base: number; readonly layout: Layout };

export function readStaged(v: unknown): Staged | null {
  if (!record(v) || !text(v["id"]) || !count(v["base"])) {
    return null;
  }
  const next = readLayout(v["layout"]);
  return next === null ? null : { id: v["id"], base: v["base"], layout: next };
}

export function readLayout(v: unknown): Layout | null {
  return layout(v) ?? null;
}

/** The sets written in `text`, or null when a line names no node. Blank lines are skipped. */
export function parseSets(text: string): string[][] | null {
  const lines = text.split("\n").filter((line) => line.trim() !== "");
  const sets = lines.map((line) => line.split(/[\s,]+/).filter((node) => node !== ""));
  return sets.length === 0 || sets.some((set) => set.length === 0) ? null : sets;
}
