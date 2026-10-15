//! What kind of file a key holds, read from the content type its write stored,
//! so a listing can say "image" or "archive" without opening the object.

import type { IconName } from "./icons.ts";

export type FileKind = { readonly icon: IconName; readonly label: string };

const FILE: FileKind = { icon: "file", label: "File" };
const TEXT: FileKind = { icon: "text", label: "Text" };
const ARCHIVE: FileKind = { icon: "archive", label: "Archive" };

/** Families named by the type's first half. */
const FAMILIES: Readonly<Record<string, FileKind>> = {
  image: { icon: "image", label: "Image" },
  video: { icon: "video", label: "Video" },
  audio: { icon: "audio", label: "Audio" },
  text: TEXT,
};

/** Application types that are text a reader can open. */
const READABLE = new Set(["application/json", "application/xml", "application/javascript"]);

/** Application types that pack other files. */
const PACKED = new Set(["application/zip", "application/gzip", "application/x-gzip", "application/x-tar", "application/x-7z-compressed"]);

/** The kind for a stored `Content-Type`, parameters and case ignored; no type is a plain file. */
export function kindOf(contentType: string | null): FileKind {
  const essence = (contentType ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
  const [family, subtype = ""] = essence.split("/");
  const named = family === undefined ? undefined : FAMILIES[family];
  if (named !== undefined) {
    return named;
  }
  if (family !== "application") {
    return FILE;
  }
  if (READABLE.has(essence) || subtype.endsWith("+json") || subtype.endsWith("+xml")) {
    return TEXT;
  }
  return PACKED.has(essence) ? ARCHIVE : FILE;
}
