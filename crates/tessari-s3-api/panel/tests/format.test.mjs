import { test } from "node:test";
import assert from "node:assert/strict";
import { size } from "../src/format.ts";

test("sizes are binary units with one decimal above a KiB", () => {
  assert.equal(size(0), "0 B");
  assert.equal(size(1023), "1,023 B");
  assert.equal(size(1024), "1.0 KiB");
  assert.equal(size(5 * 1024 * 1024 + 300 * 1024), "5.3 MiB");
  assert.equal(size(3 * 1024 ** 4), "3.0 TiB");
});
