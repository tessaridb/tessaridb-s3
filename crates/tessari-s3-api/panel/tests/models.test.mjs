import { test } from "node:test";
import assert from "node:assert/strict";
import { readCapabilities, readCluster, readIssued, readSpaces, readStatus, readUsage, readUsers } from "../src/models.ts";

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
  const may = { access_key_id: "TSABCDEFGHIJKLMNOPQR", operate: false, administer: true, view_cluster: false };
  assert.deepEqual(readCapabilities(may), may);
  assert.equal(readCapabilities({ ...may, operate: "no" }), null);
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
