import { test } from "node:test";
import assert from "node:assert/strict";
import { KINDS, outputOf, readMediaRules, readMediaSet, readSweep, spell, sweepText, typesOf } from "../src/media-model.ts";

const rule = { id: "photos", content_types: ["image/*"], prefix: "photos/", outputs: ["image/320/jpeg", "image/1920/webp"] };

test("media rules read as the server sends them", () => {
  assert.deepEqual(readMediaRules({ rules: [rule] }), [rule]);
  assert.deepEqual(readMediaRules({ rules: [] }), []);
});

test("a rule missing a field, or naming an output off the lists, is not a rule", () => {
  const { prefix: _gone, ...noPrefix } = rule;
  assert.equal(readMediaRules({ rules: [noPrefix] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, outputs: ["thumb"] }] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, outputs: ["image/15/jpeg"] }] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, content_types: "image/*" }] }), null);
  assert.equal(readMediaRules({}), null);
});

test("an output is spelled kind/size/format and read back the same", () => {
  assert.deepEqual(outputOf("video/720p/mp4"), { kind: "video", size: "720p", format: "mp4", quality: "" });
  assert.equal(spell({ kind: "poster", size: "640", format: "avif", quality: "" }), "poster/640/avif");
  assert.equal(outputOf("video/720/mp4"), null);
  assert.equal(outputOf("image/320/mp4"), null);
});

test("an image or a poster takes any side, a box with a mode, and a quality on a lossy format", () => {
  assert.deepEqual(outputOf("image/1300/webp@70"), { kind: "image", size: "1300", format: "webp", quality: "70" });
  assert.deepEqual(outputOf("poster/400x400-cover/avif"), { kind: "poster", size: "400x400-cover", format: "avif", quality: "" });
  for (const name of ["image/16/jpeg", "image/7680/png", "image/800x600/jpeg@100", "image/800x600-exact/png", "image/300x200-fit/webp@1"]) {
    const output = outputOf(name);
    assert.notEqual(output, null, name);
    assert.equal(output === null ? null : spell(output), name);
  }
  for (const name of ["image/15/jpeg", "image/7681/jpeg", "image/640-cover/jpeg", "image/800x/jpeg", "image/800x600-zoom/jpeg", "image/640/png@80", "image/640/jpeg@0", "image/640/jpeg@101", "image/640/jpeg@", "video/720p/mp4@80"]) {
    assert.equal(outputOf(name), null, name);
  }
});

test("the lists are the server's", () => {
  assert.deepEqual(Object.keys(KINDS), ["image", "video", "poster"]);
  assert.deepEqual(KINDS.image.sizes, ["150", "320", "640", "768", "1024", "1280", "1440", "1920", "2560", "3840"]);
  assert.deepEqual(KINDS.video.sizes, ["360p", "480p", "720p", "1080p", "1440p", "2160p"]);
  assert.deepEqual(KINDS.video.formats, ["mp4", "webm", "hls"]);
  assert.deepEqual(KINDS.poster.formats, ["jpeg", "png", "webp", "avif"]);
});

test("content types are written comma-separated and sent trimmed, without empties", () => {
  assert.deepEqual(typesOf(" image/* , video/mp4,, "), ["image/*", "video/mp4"]);
  assert.deepEqual(typesOf("  "), []);
});

test("a sweep reads as the server sends it and says where it stands", () => {
  assert.deepEqual(readSweep({ state: "running", listed: 100, queued: 6 }), { state: "running", listed: 100, queued: 6 });
  assert.equal(readSweep({ state: "paused", listed: 0, queued: 0 }), null);
  assert.equal(readSweep({ state: "done", listed: -1, queued: 0 }), null);
  assert.equal(readSweep({ state: "done", listed: 4 }), null);
  assert.equal(sweepText({ state: "done", listed: 104, queued: 6 }), "The objects already here were rendered: 104 listed, 6 renditions queued.");
  assert.match(sweepText({ state: "none", listed: 0, queued: 0 }), /not been rendered/);
});

test("an HLS stream names its rungs once, ascending", () => {
  assert.deepEqual(outputOf("video/360p+720p/hls"), { kind: "video", size: "360p+720p", format: "hls", quality: "" });
  assert.notEqual(outputOf("video/1080p/hls"), null);
  for (const name of ["video/720p+360p/hls", "video/720p+720p/hls", "video/700p/hls", "video/360p+/hls", "video/360p+720p/mp4"]) {
    assert.equal(outputOf(name), null, name);
  }
});

test("the rules come with the version a save names", () => {
  assert.deepEqual(readMediaSet({ rules: [rule], version: 3 }), { rules: [rule], version: 3 });
  assert.equal(readMediaSet({ rules: [rule] }), null);
  assert.equal(readMediaSet({ rules: [rule], version: -1 }), null);
});
