import { test } from "node:test";
import assert from "node:assert/strict";
import { readBuckets, readCapabilities, readCluster, readIssued, readSpaces, readStatus, readUploadKey, readUsage, readUsers } from "../src/models.ts";

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
  const media = { bucket: "media", objects: 2, bytes: 30, inline_bytes: 10, raw_bytes: 60 };
  const taken = { taken: "2026-10-08T10:00:00.000Z", buckets: [media], objects: 2, bytes: 30, inline_bytes: 10, raw_bytes: 60 };
  assert.deepEqual(readUsage(taken), taken);
  const none = { taken: null, buckets: [], objects: 0, bytes: 0, inline_bytes: 0, raw_bytes: 0 };
  assert.deepEqual(readUsage(none), none);
  assert.equal(readUsage({ ...none, buckets: [{ ...media, objects: "2" }] }), null);
  assert.equal(readUsage({ ...taken, buckets: [{ bucket: "media", objects: 2, bytes: 30 }] }), null, "a bucket without its occupancy");
  assert.equal(readUsage({ taken: null, buckets: [], objects: 0, bytes: 0 }), null, "totals without the occupancy");
});

const ann = { name: "ann", space: "alpha", role: "member", create_buckets: true, operator: false, cluster_viewer: false, disabled: false, created: "2026-10-08T10:00:00.000Z" };

test("users read with their role, their flags and whether they are disabled", () => {
  assert.deepEqual(readUsers({ users: [ann] }), [ann]);
  assert.equal(readUsers({ users: [{ ...ann, role: "root" }] }), null, "a role the server does not have");
  assert.equal(readUsers({ users: [{ ...ann, disabled: "no" }] }), null);
  const { operator: _gone, ...withoutOperator } = ann;
  assert.equal(readUsers({ users: [withoutOperator] }), null);
});

test("spaces read with their creation time", () => {
  const spaces = { spaces: [{ name: "alpha", created: "2026-10-08T10:00:00.000Z" }] };
  assert.deepEqual(readSpaces(spaces), spaces.spaces);
  assert.equal(readSpaces({ spaces: [{ name: "alpha" }] }), null);
});

test("an issued key carries its id and its secret, both as text", () => {
  const key = { access_key_id: "TSABCDEFGHIJKLMNOPQR", secret_access_key: "s".repeat(40) };
  assert.deepEqual(readIssued(key), key);
  assert.equal(readIssued({ access_key_id: "TSABCDEFGHIJKLMNOPQR" }), null);
});

test("the session says what the key may do, each as a yes or no", () => {
  const may = { access_key_id: "TSABCDEFGHIJKLMNOPQR", operate: false, administer: true, view_cluster: false, issue_upload_keys: true };
  assert.deepEqual(readCapabilities(may), may);
  assert.equal(readCapabilities({ ...may, operate: "no" }), null);
  const { issue_upload_keys: _gone, ...older } = may;
  assert.equal(readCapabilities(older), null, "every capability is always sent");
});

test("the cluster view reads members, a layout or none, the backlog and the metadata nodes", () => {
  const cluster = {
    node: "n1",
    erasure: "2+1",
    members: [{ node: "n1", endpoint: "127.0.0.1:1", answering: true, drive }],
    layout: { version: 1, nodes: ["n1", "n2", "n3"] },
    heal_backlog: { listed: 0, more: false },
    metadata: { addresses: ["127.0.0.1:9080"] },
  };
  assert.deepEqual(readCluster(cluster), cluster);
  assert.deepEqual(readCluster({ ...cluster, node: null, erasure: null, members: null, layout: null })?.layout, null);
  assert.equal(readCluster({ ...cluster, metadata: { addresses: [1] } }), null);
});

test("a bucket carries its limits, null where it has none", () => {
  const row = { name: "media", created: "2026-10-08T00:00:00.000Z", region: "us-east-1" };
  assert.deepEqual(readBuckets({ buckets: [{ ...row, max_bytes: 1000, max_objects: null }] }), [
    { ...row, maxBytes: 1000, maxObjects: null },
  ]);
  assert.equal(readBuckets({ buckets: [{ ...row, max_bytes: -1, max_objects: null }] }), null);
  assert.equal(readBuckets({ buckets: [row] }), null, "the limits are always sent");
});

test("an upload key carries its id, its secret and when it stops", () => {
  const key = { access_key_id: "TSABCDEFGHIJKLMNOPQR", secret_access_key: "s".repeat(40), expires: "2026-10-08T11:00:00.000Z" };
  assert.deepEqual(readUploadKey(key), key);
  const { expires: _gone, ...withoutExpiry } = key;
  assert.equal(readUploadKey(withoutExpiry), null);
});
