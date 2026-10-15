//! A bucket's media rules as the console sends and answers them: which uploads
//! (content types under a key prefix) get which renditions. An output is
//! `kind/size/format`, each picked from the server's lists; the server checks a
//! rule set as a whole, this page only reads the answer and shapes the form.

import { list, record, text } from "./models.ts";

const IMAGE_SIZES = ["150", "320", "640", "768", "1024", "1280", "1440", "1920", "2560", "3840"] as const;
const STILL_FORMATS = ["jpeg", "png", "webp", "avif"] as const;

/** The kinds of rendition, each with the sizes and formats the server makes it in. */
export const KINDS = {
  image: { label: "Image", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "image" },
  video: { label: "Video", sizes: ["360p", "480p", "720p", "1080p", "1440p", "2160p"], formats: ["mp4", "webm"], from: "video" },
  poster: { label: "Poster frame", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "video" },
} as const;

export type Kind = keyof typeof KINDS;

export type Output = { readonly kind: Kind; readonly size: string; readonly format: string };

export type MediaRule = {
  readonly id: string;
  readonly content_types: readonly string[];
  readonly prefix: string;
  readonly outputs: readonly string[];
};

export const isKind = (value: string): value is Kind => Object.hasOwn(KINDS, value);

/** The output spelled `name`, when every part of it is on the server's lists. */
export function outputOf(name: string): Output | null {
  const [kind, size, format, extra] = name.split("/");
  if (kind === undefined || size === undefined || format === undefined || extra !== undefined || !isKind(kind)) {
    return null;
  }
  const lists: { readonly sizes: readonly string[]; readonly formats: readonly string[] } = KINDS[kind];
  return lists.sizes.includes(size) && lists.formats.includes(format) ? { kind, size, format } : null;
}

/** The spelling the server takes. */
export const spell = (output: Output): string => `${output.kind}/${output.size}/${output.format}`;

function rule(v: unknown): MediaRule | null {
  if (!record(v) || !text(v["id"]) || !text(v["prefix"])) {
    return null;
  }
  const types = list(v["content_types"], (entry) => (text(entry) ? entry : null));
  const outputs = list(v["outputs"], (entry) => (text(entry) && outputOf(entry) !== null ? entry : null));
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
