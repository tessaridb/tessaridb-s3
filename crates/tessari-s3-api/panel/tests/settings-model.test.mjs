import { test } from "node:test";
import assert from "node:assert/strict";
import { readSettings } from "../src/settings-model.ts";

const plain = { name: "TESSARIDB_S3_RECLAIM_INTERVAL_SECS", scope: "region", class: "setting", value: "45", stored: true, applies: "live", fixed: false };
const secret = { name: "TESSARIDB_S3_IAM_KEY", scope: "global", class: "secret", value: null, stored: true, applies: "start", fixed: false };

test("the stored settings are read with each row's class, scope and when it applies", () => {
  assert.deepEqual(readSettings({ region: "us-east-1", settings: [plain, secret] }), {
    region: "us-east-1",
    settings: [
      { name: "TESSARIDB_S3_RECLAIM_INTERVAL_SECS", scope: "region", secret: false, value: "45", stored: true, live: true, fixed: false },
      { name: "TESSARIDB_S3_IAM_KEY", scope: "global", secret: true, value: null, stored: true, live: false, fixed: false },
    ],
  });
});

test("a secret carrying a value, or a row nobody sends, is refused", () => {
  assert.equal(readSettings({ region: "us-east-1", settings: [{ ...secret, value: "abc" }] }), null);
  assert.equal(readSettings({ region: "us-east-1", settings: [{ ...plain, class: "other" }] }), null);
  assert.equal(readSettings({ region: "us-east-1", settings: [{ ...plain, applies: "soon" }] }), null);
  assert.equal(readSettings({ region: "us-east-1", settings: [{ ...plain, fixed: "no" }] }), null);
  assert.equal(readSettings({ settings: [plain] }), null);
  assert.equal(readSettings(null), null);
});
