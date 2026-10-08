import { test } from "node:test";
import assert from "node:assert/strict";
import { readStatus } from "../src/models.ts";

const status = (members) => ({ version: "0.0.0", node: "n1", region: "us-east-1", erasure: "2+1", members, heal_backlog: { listed: 0, more: false } });

test("a member says whether it answered", () => {
  const read = readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", answering: true }, { node: "n2", endpoint: "127.0.0.1:2", answering: false }]));
  assert.deepEqual(read?.members, [
    { node: "n1", endpoint: "127.0.0.1:1", answering: true },
    { node: "n2", endpoint: "127.0.0.1:2", answering: false },
  ]);
});

test("a member without its answer is not a status", () => {
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1" }])), null);
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", answering: "yes" }])), null);
});
