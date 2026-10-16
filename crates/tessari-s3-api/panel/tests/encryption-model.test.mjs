import { test } from "node:test";
import assert from "node:assert/strict";
import { readKeys } from "../src/encryption-model.ts";

test("the keys are read in each of their three states", () => {
  assert.deepEqual(readKeys({ state: "uninitialised", active: null, keys: null }), { state: "uninitialised" });
  assert.deepEqual(readKeys({ state: "sealed", active: null, keys: null }), { state: "sealed" });
  assert.deepEqual(readKeys({ state: "unsealed", active: "server_1", keys: 2, named: [] }), { state: "unsealed", active: "server_1", keys: 2, named: [] });
  assert.deepEqual(readKeys({ state: "unsealed", active: "server_1", keys: 3, named: ["tenant-7"] }), {
    state: "unsealed",
    active: "server_1",
    keys: 3,
    named: ["tenant-7"],
  });
});

test("an unsealed answer without its key, or a state nobody sends, is refused", () => {
  assert.equal(readKeys({ state: "unsealed", active: null, keys: 1, named: [] }), null);
  assert.equal(readKeys({ state: "unsealed", active: "server_1", keys: 1 }), null);
  assert.equal(readKeys({ state: "unsealed", active: "server_1", keys: 2, named: [7] }), null);
  assert.equal(readKeys({ state: "unsealed", active: "server_1", keys: -1 }), null);
  assert.equal(readKeys({ state: "open", active: null, keys: null }), null);
  assert.equal(readKeys(null), null);
});
