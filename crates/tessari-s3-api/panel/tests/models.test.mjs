import { test } from "node:test";
import assert from "node:assert/strict";
import { readStatus, readUsage } from "../src/models.ts";

const drive = { capacity: 1000, free: 400, available: 300 };
const status = (members, own = drive) => ({ version: "0.0.0", node: "n1", region: "us-east-1", erasure: "2+1", members, heal_backlog: { listed: 0, more: false }, drive: own });

test("a member says whether it answered and how full its drive is", () => {
  const read = readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", answering: true, drive }, { node: "n2", endpoint: "127.0.0.1:2", answering: false, drive: null }]));
  assert.deepEqual(read?.members, [
    { node: "n1", endpoint: "127.0.0.1:1", answering: true, drive },
    { node: "n2", endpoint: "127.0.0.1:2", answering: false, drive: null },
  ]);
  assert.deepEqual(read?.drive, drive);
});

test("a member without its answer is not a status", () => {
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", drive: null }])), null);
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", answering: "yes", drive: null }])), null);
});

test("a drive must carry three byte counts, or be null", () => {
  assert.equal(readStatus(status(null, { capacity: 1000, free: -1, available: 0 })), null);
  const { drive: _absent, ...withoutDrive } = status(null);
  assert.equal(readStatus(withoutDrive), null);
  assert.deepEqual(readStatus(status(null, null))?.drive, null);
});

test("usage reads buckets with their figures, and a measurement never taken", () => {
  const taken = { taken: "2026-10-08T10:00:00.000Z", buckets: [{ bucket: "media", objects: 2, bytes: 30 }], objects: 2, bytes: 30 };
  assert.deepEqual(readUsage(taken), taken);
  assert.deepEqual(readUsage({ taken: null, buckets: [], objects: 0, bytes: 0 }), { taken: null, buckets: [], objects: 0, bytes: 0 });
  assert.equal(readUsage({ taken: null, buckets: [{ bucket: "media", objects: "2", bytes: 30 }], objects: 0, bytes: 0 }), null);
});
