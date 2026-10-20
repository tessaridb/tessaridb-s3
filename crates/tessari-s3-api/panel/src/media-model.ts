//! A bucket's media rules as the console sends and answers them: which uploads
//! (content types under a key prefix) get which renditions. An output is
//! `kind/size/format`: a video's size and every format come from the server's
//! lists; an image's or a poster's size is a side or a box (`400x400-cover`)
//! of 16-7680 pixels, and a lossy still format may carry `@quality`. The server
//! checks a rule set as a whole, this page only reads the answer and shapes the form.

import { list, record, text } from "./models.ts";

const IMAGE_SIZES = ["150", "320", "640", "768", "1024", "1280", "1440", "1920", "2560", "3840"] as const;
const STILL_FORMATS = ["jpeg", "png", "webp", "avif"] as const;

/** The kinds of rendition, each with the formats the server makes it in and its sizes: a video's list, or an image's common sides. */
export const KINDS = {
  image: { label: "Image", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "image" },
  video: { label: "Video", sizes: ["360p", "480p", "720p", "1080p", "1440p", "2160p"], formats: ["mp4", "webm"], from: "video" },
  poster: { label: "Poster frame", sizes: IMAGE_SIZES, formats: STILL_FORMATS, from: "video" },
} as const;

export type Kind = keyof typeof KINDS;

export type Output = { readonly kind: Kind; readonly size: string; readonly format: string; readonly quality: string };

export type MediaRule = {
  readonly id: string;
  readonly content_types: readonly string[];
  readonly prefix: string;
  readonly outputs: readonly string[];
};

export const isKind = (value: string): value is Kind => Object.hasOwn(KINDS, value);

/** A side of 16-7680 pixels, digits only. */
const isSide = (written: string): boolean => /^[0-9]+$/.test(written) && Number(written) >= 16 && Number(written) <= 7680;

/** A still size: a side, or a box `WxH` with an optional `-fit`, `-cover` or `-exact`. */
export function isStillSize(written: string): boolean {
  const box = /^([0-9]+)x([0-9]+)(-(fit|cover|exact))?$/.exec(written);
  return box === null ? isSide(written) : isSide(box[1] ?? "") && isSide(box[2] ?? "");
}

/** A quality: empty for the format's own, else 1-100 on a lossy format. */
const isQuality = (written: string, format: string): boolean =>
  written === "" || (format !== "png" && /^[0-9]+$/.test(written) && Number(written) >= 1 && Number(written) <= 100);

/** The output spelled `name`, when the server would take it. */
export function outputOf(name: string): Output | null {
  const [kind, size, written, extra] = name.split("/");
  if (kind === undefined || size === undefined || written === undefined || extra !== undefined || !isKind(kind)) {
    return null;
  }
  const at = written.indexOf("@");
  const format = at < 0 ? written : written.slice(0, at);
  const quality = at < 0 ? "" : written.slice(at + 1);
  const lists: { readonly sizes: readonly string[]; readonly formats: readonly string[] } = KINDS[kind];
  if (!lists.formats.includes(format) || (at >= 0 && quality === "")) {
    return null;
  }
  if (kind === "video") {
    return lists.sizes.includes(size) && at < 0 ? { kind, size, format, quality } : null;
  }
  return isStillSize(size) && isQuality(quality, format) ? { kind, size, format, quality } : null;
}

/** The spelling the server takes. */
export const spell = (output: Output): string =>
  `${output.kind}/${output.size}/${output.format}${output.quality === "" ? "" : `@${output.quality}`}`;

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
