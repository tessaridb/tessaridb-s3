//! A bucket's media rules as the console sends and answers them: which uploads
//! (content types under a key prefix) get which renditions. The server checks a
//! rule set as a whole; this page only reads the answer and shapes the form.

import { list, record, text } from "./models.ts";

/** The renditions the server makes, with the words the form shows for each. */
export const OUTPUTS = [
  { name: "thumb", label: "Thumbnail", kind: "image" },
  { name: "web", label: "Web image", kind: "image" },
  { name: "720p", label: "720p video", kind: "video" },
  { name: "poster", label: "Video poster", kind: "video" },
] as const;

export type Output = (typeof OUTPUTS)[number]["name"];

export type MediaRule = {
  readonly id: string;
  readonly content_types: readonly string[];
  readonly prefix: string;
  readonly outputs: readonly Output[];
};

const isOutput = (value: unknown): value is Output => OUTPUTS.some((output) => output.name === value);

function rule(v: unknown): MediaRule | null {
  if (!record(v) || !text(v["id"]) || !text(v["prefix"])) {
    return null;
  }
  const types = list(v["content_types"], (entry) => (text(entry) ? entry : null));
  const outputs = list(v["outputs"], (entry) => (isOutput(entry) ? entry : null));
  return types === null || outputs === null ? null : { id: v["id"], content_types: types, prefix: v["prefix"], outputs };
}

export function readMediaRules(v: unknown): readonly MediaRule[] | null {
  return record(v) ? list(v["rules"], rule) : null;
}

/** The content types an operator wrote comma-separated, trimmed, empties dropped. */
export const typesOf = (written: string): string[] =>
  written
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part !== "");
