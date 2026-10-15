import { test } from "node:test";
import assert from "node:assert/strict";
import { readIssue, readKeys } from "../src/keys-model.ts";

const id = "TSABCDEFGHIJKLMNOPQR";

test("an issue answers the new key, or the key the form's token already issued and no secret", () => {
  const key = { access_key_id: id, secret_access_key: "s".repeat(40) };
  assert.deepEqual(readIssue(key), { kind: "issued", key });
  assert.deepEqual(readIssue({ access_key_id: id, already_issued: true }), { kind: "already", access_key_id: id });
  assert.equal(readIssue({ access_key_id: id, already_issued: false }), null);
  assert.equal(readIssue({ access_key_id: id }), null);
});

test("a user's keys carry an id, when, whether disabled, and an upload key's reach", () => {
  const full = { access_key_id: id, created: "2026-10-09T10:00:00.000Z", disabled: false, upload: null };
  const upload = { ...full, access_key_id: "TSBBCDEFGHIJKLMNOPQR", upload: { bucket: "media", key: "a.png", expires: 1760000000 } };
  assert.deepEqual(readKeys({ keys: [full, upload] }), [full, upload]);
  assert.deepEqual(readKeys({ keys: [] }), []);
  assert.equal(readKeys({ keys: [{ ...full, disabled: "no" }] }), null);
  assert.equal(readKeys({ keys: [{ ...full, upload: { bucket: "media", key: "a.png" } }] }), null, "an upload reach without its expiry");
  const { upload: _gone, ...older } = full;
  assert.equal(readKeys({ keys: [older] }), null, "the reach is always sent, null for a full key");
});
