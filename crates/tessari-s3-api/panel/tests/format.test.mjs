import { test } from "node:test";
import assert from "node:assert/strict";
import { bytesOf, size } from "../src/format.ts";

test("sizes are binary units with one decimal above a KiB", () => {
  assert.equal(size(0), "0 B");
  assert.equal(size(1023), "1,023 B");
  assert.equal(size(1024), "1.0 KiB");
  assert.equal(size(5 * 1024 * 1024 + 300 * 1024), "5.3 MiB");
  assert.equal(size(3 * 1024 ** 4), "3.0 TiB");
});

test("a quota amount in a binary unit becomes bytes; empty is no limit", () => {
  assert.equal(bytesOf("10", "GiB"), 10 * 1024 ** 3);
  assert.equal(bytesOf("1.5", "MiB"), 1572864);
  assert.equal(bytesOf(" ", "GiB"), null);
  for (const wrong of ["-1", "abc", "1e3", "0.0000000000001", "99999999999", "1,5"]) {
    assert.equal(bytesOf(wrong, "TiB"), undefined, wrong);
  }
});
