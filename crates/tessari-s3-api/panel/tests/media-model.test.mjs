import { test } from "node:test";
import assert from "node:assert/strict";
import { OUTPUTS, readMediaRules, typesOf } from "../src/media-model.ts";

const rule = { id: "photos", content_types: ["image/*"], prefix: "photos/", outputs: ["thumb", "web"] };

test("media rules read as the server sends them", () => {
  assert.deepEqual(readMediaRules({ rules: [rule] }), [rule]);
  assert.deepEqual(readMediaRules({ rules: [] }), []);
});

test("a rule missing a field, or naming an output this page does not know, is not a rule", () => {
  const { prefix: _gone, ...noPrefix } = rule;
  assert.equal(readMediaRules({ rules: [noPrefix] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, outputs: ["gif"] }] }), null);
  assert.equal(readMediaRules({ rules: [{ ...rule, content_types: "image/*" }] }), null);
  assert.equal(readMediaRules({}), null);
});

test("content types are written comma-separated and sent trimmed, without empties", () => {
  assert.deepEqual(typesOf(" image/* , video/mp4,, "), ["image/*", "video/mp4"]);
  assert.deepEqual(typesOf("  "), []);
});

test("the outputs are the server's four presets", () => {
  assert.deepEqual(OUTPUTS.map((output) => output.name), ["thumb", "web", "720p", "poster"]);
});
