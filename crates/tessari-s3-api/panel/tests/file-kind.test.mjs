import { test } from "node:test";
import assert from "node:assert/strict";
import { kindOf } from "../src/file-kind.ts";

test("a content type names the kind of file by its family", () => {
  assert.deepEqual(kindOf("image/png"), { icon: "image", label: "Image" });
  assert.deepEqual(kindOf("video/mp4"), { icon: "video", label: "Video" });
  assert.deepEqual(kindOf("audio/mpeg"), { icon: "audio", label: "Audio" });
  assert.deepEqual(kindOf("text/plain"), { icon: "text", label: "Text" });
});

test("structured text and archives are told apart from other application types", () => {
  for (const type of ["application/json", "application/xml", "application/javascript", "application/ld+json", "image/svg+xml"]) {
    assert.equal(kindOf(type).label, type === "image/svg+xml" ? "Image" : "Text", type);
  }
  for (const type of ["application/zip", "application/gzip", "application/x-gzip", "application/x-tar", "application/x-7z-compressed"]) {
    assert.deepEqual(kindOf(type), { icon: "archive", label: "Archive" }, type);
  }
  assert.deepEqual(kindOf("application/pdf"), { icon: "file", label: "File" });
  assert.deepEqual(kindOf("application/octet-stream"), { icon: "file", label: "File" });
});

test("parameters and case do not change the kind, and no type is a plain file", () => {
  assert.equal(kindOf("Text/HTML; charset=utf-8").label, "Text");
  assert.equal(kindOf(" IMAGE/JPEG ").label, "Image");
  assert.deepEqual(kindOf(null), { icon: "file", label: "File" });
  assert.deepEqual(kindOf("textual/thing"), { icon: "file", label: "File" }, "a family is matched whole, not by prefix");
});
