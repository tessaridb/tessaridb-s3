import { test } from "node:test";
import assert from "node:assert/strict";
import { KINDS, outputOf, readMediaRules, spell, typesOf } from "../src/media-model.ts";

const rule = { id: "photos", content_types: ["image/*"], prefix: "photos/", outputs: ["image/320/jpeg", "image/1920/webp"] };

test("media rules read as the server sends them", () => {
  assert.deepEqual(readMediaRules({ rules: [rule] }), [rule]);
  assert.deepEqual(readMediaRules({ rules: [] }), []);
});

test("a rule missing a field, or naming an output off the lists, is not a rule", () => {
  const { prefix: _gone, ...noPrefix } = rule;
  assert.equal(readMediaRules({ rules: [noPrefix] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, outputs: ["thumb"] }] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, outputs: ["image/1300/jpeg"] }] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, content_types: "image/*" }] }), null);
  assert.equal(readMediaRules({}), null);
});

test("an output is spelled kind/size/format and read back the same", () => {
  assert.deepEqual(outputOf("video/720p/mp4"), { kind: "video", size: "720p", format: "mp4" });
  assert.equal(spell({ kind: "poster", size: "640", format: "avif" }), "poster/640/avif");
  assert.equal(outputOf("video/720/mp4"), null);
  assert.equal(outputOf("image/320/mp4"), null);
});

test("the lists are the server's", () => {
  assert.deepEqual(Object.keys(KINDS), ["image", "video", "poster"]);
  assert.deepEqual(KINDS.image.sizes, ["150", "320", "640", "768", "1024", "1280", "1440", "1920", "2560", "3840"]);
  assert.deepEqual(KINDS.video.sizes, ["360p", "480p", "720p", "1080p", "1440p", "2160p"]);
  assert.deepEqual(KINDS.video.formats, ["mp4", "webm"]);
  assert.deepEqual(KINDS.poster.formats, ["jpeg", "png", "webp", "avif"]);
});

test("content types are written comma-separated and sent trimmed, without empties", () => {
  assert.deepEqual(typesOf(" image/* , video/mp4,, "), ["image/*", "video/mp4"]);
  assert.deepEqual(typesOf("  "), []);
});
