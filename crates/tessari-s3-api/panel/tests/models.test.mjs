import { test } from "node:test";
import assert from "node:assert/strict";
import { readBuckets, readCapabilities, readDetail, readFailedRenditions, readCluster, readIssued, readListing, readSpaces, readStatus, readUploadKey, readUsage, readUsers } from "../src/models.ts";

const drive = { capacity: 1000, free: 400, available: 300 };
const status = (members, own = drive) => ({ version: "0.0.0", node: "n1", region: "us-east-1", erasure: "2+1", members, heal_backlog: { listed: 0, more: false, at_risk: 0 }, replication: null, drive: own });

test("a member says whether it answered and how full its drive is", () => {
  const read = readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", zone: "eu-1a", answering: true, drive }, { node: "n2", endpoint: "127.0.0.1:2", zone: null, answering: false, drive: null }]));
  assert.deepEqual(read?.members, [
    { node: "n1", endpoint: "127.0.0.1:1", zone: "eu-1a", answering: true, drive },
    { node: "n2", endpoint: "127.0.0.1:2", zone: null, answering: false, drive: null },
  ]);
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", answering: true, drive }])), null, "the zone is always sent, null when none");
  assert.deepEqual(read?.drive, drive);
});

test("a member without its answer is not a status", () => {
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", zone: null, drive: null }])), null);
  assert.equal(readStatus(status([{ node: "n1", endpoint: "127.0.0.1:1", zone: null, answering: "yes", drive: null }])), null);
});

test("a drive must carry three byte counts, or be null", () => {
  assert.equal(readStatus(status(null, { capacity: 1000, free: -1, available: 0 })), null);
  const { drive: _absent, ...withoutDrive } = status(null);
  assert.equal(readStatus(withoutDrive), null);
  assert.deepEqual(readStatus(status(null, null))?.drive, null);
});

test("usage reads buckets with their figures, and a measurement never taken", () => {
  const media = { bucket: "media", objects: 2, bytes: 30, raw_bytes: 60 };
  const taken = { taken: "2026-10-08T10:00:00.000Z", stale: false, buckets: [media], objects: 2, bytes: 30, raw_bytes: 60 };
  assert.deepEqual(readUsage(taken), taken);
  const none = { taken: null, stale: true, buckets: [], objects: 0, bytes: 0, raw_bytes: 0 };
  assert.deepEqual(readUsage(none), none);
  assert.equal(readUsage({ ...none, buckets: [{ ...media, objects: "2" }] }), null);
  assert.equal(readUsage({ ...taken, buckets: [{ bucket: "media", objects: 2, bytes: 30 }] }), null, "a bucket without its occupancy");
  assert.equal(readUsage({ taken: null, stale: true, buckets: [], objects: 0, bytes: 0 }), null, "totals without the occupancy");
  const { stale: _, ...unflagged } = taken;
  assert.equal(readUsage(unflagged), null, "a measurement that does not say whether it is too old for quotas");
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
  const may = { access_key_id: "TSABCDEFGHIJKLMNOPQR", operate: false, administer: true, view_cluster: false, issue_upload_keys: true, space: "alpha" };
  assert.deepEqual(readCapabilities(may), may);
  assert.deepEqual(readCapabilities({ ...may, space: null }), { ...may, space: null }, "root belongs to no space");
  assert.equal(readCapabilities({ ...may, space: 7 }), null);
  assert.equal(readCapabilities({ ...may, operate: "no" }), null);
  const { issue_upload_keys: _gone, ...older } = may;
  assert.equal(readCapabilities(older), null, "every capability is always sent");
});

test("the cluster view reads members, a layout or none, the backlog and the metadata nodes", () => {
  const cluster = {
    node: "n1",
    erasure: "2+1",
    members: [{ node: "n1", endpoint: "127.0.0.1:1", zone: "a", answering: true, drive }],
    layout: { version: 2, sets: [[{ node: "n1", zone: "a" }, { node: "n2", zone: "b" }, { node: "n3", zone: null }], [{ node: "n4", zone: "a" }, { node: "n5", zone: "b" }, { node: "n6", zone: "c" }]] },
    heal_backlog: { listed: 3, more: false, at_risk: 1 },
    moving: { listed: 2, more: true },
    metadata: { addresses: ["127.0.0.1:9080"] },
  };
  assert.deepEqual(readCluster(cluster), cluster);
  assert.deepEqual(readCluster({ ...cluster, node: null, erasure: null, members: null, layout: null })?.layout, null);
  assert.equal(readCluster({ ...cluster, metadata: { addresses: [1] } }), null);
  assert.equal(readCluster({ ...cluster, layout: { version: 1, nodes: ["n1", "n2", "n3"] } }), null, "a layout is its sets");
  assert.equal(readCluster({ ...cluster, layout: { version: 1, sets: [[{ node: "n1" }]] } }), null, "a set member always says its zone");
  assert.equal(readCluster({ ...cluster, heal_backlog: { listed: 3, more: false } }), null, "the at-risk count is always sent");
  assert.equal(readCluster({ ...cluster, moving: { listed: 2 } }), null, "the objects still to move are always sent");
});

test("the status backlog carries how many objects are at risk", () => {
  const read = readStatus({ ...status(null), heal_backlog: { listed: 4, more: true, at_risk: 2 } });
  assert.deepEqual(read?.heal_backlog, { listed: 4, more: true, at_risk: 2 });
  assert.equal(readStatus({ ...status(null), heal_backlog: { listed: 4, more: true } }), null);
});

test("a bucket carries its limits, null where it has none", () => {
  const row = { name: "media", created: "2026-10-08T00:00:00.000Z", region: "us-east-1", acknowledgement: "region", encryption: null };
  assert.deepEqual(readBuckets({ buckets: [{ ...row, max_bytes: 1000, max_objects: null }] }), [
    { ...row, maxBytes: 1000, maxObjects: null },
  ]);
  const { acknowledgement: _, ...silent } = row;
  assert.equal(readBuckets({ buckets: [{ ...silent, max_bytes: 1000, max_objects: null }] }), null, "the level is always sent");
  assert.equal(readBuckets({ buckets: [{ ...row, acknowledgement: "leader", max_bytes: 1000, max_objects: null }] }), null);
  assert.equal(readBuckets({ buckets: [{ ...row, max_bytes: -1, max_objects: null }] }), null);
  assert.equal(readBuckets({ buckets: [row] }), null, "the limits are always sent");
});

test("a bucket carries its default encryption: none, the server's key, or a named key, and whether customer keys are allowed", () => {
  const row = { name: "media", created: "2026-10-08T00:00:00.000Z", region: "us-east-1", acknowledgement: "majority", max_bytes: null, max_objects: null };
  const read = (encryption) => readBuckets({ buckets: [{ ...row, encryption }] })?.[0]?.encryption;
  assert.equal(read(null), null);
  assert.deepEqual(read({ algorithm: "AES256", key: null, customer_keys: false }), { algorithm: "AES256", customerKeys: false });
  assert.deepEqual(read({ algorithm: "aws:kms", key: "logs-key", customer_keys: true }), { algorithm: "aws:kms", key: "logs-key", customerKeys: true });
  for (const wrong of [
    { algorithm: "aws:kms", key: null, customer_keys: false },
    { algorithm: "DES", key: null, customer_keys: false },
    { algorithm: "AES256", key: "x", customer_keys: false },
    { algorithm: "AES256", key: null },
    { algorithm: "AES256", key: null, customer_keys: "yes" },
    "AES256",
    undefined,
  ]) {
    assert.equal(readBuckets({ buckets: [{ ...row, encryption: wrong }] }), null, JSON.stringify(wrong));
  }
});

test("an upload key carries its id, its secret and when it stops", () => {
  const key = { access_key_id: "TSABCDEFGHIJKLMNOPQR", secret_access_key: "s".repeat(40), expires: "2026-10-08T11:00:00.000Z" };
  assert.deepEqual(readUploadKey(key), key);
  const { expires: _gone, ...withoutExpiry } = key;
  assert.equal(readUploadKey(withoutExpiry), null);
});

test("a listing row carries the content type the write stored, null where none was", () => {
  const row = { key: "a.png", size: 10, etag: "\"e\"", modified: "2026-10-09T00:00:00.000Z" };
  const typed = { ...row, content_type: "image/png" };
  assert.deepEqual(readListing({ objects: [typed], prefixes: [], next: null })?.objects, [{ ...row, contentType: "image/png" }]);
  assert.deepEqual(readListing({ objects: [{ ...row, content_type: null }], prefixes: [], next: null })?.objects, [{ ...row, contentType: null }]);
  assert.equal(readListing({ objects: [row], prefixes: [], next: null }), null, "the type is always sent");
  assert.equal(readListing({ objects: [{ ...row, content_type: 3 }], prefixes: [], next: null }), null);
});

test("failed renditions are read with their queue, tries and last error, and a stray queue is refused", () => {
  const one = { queue: "videos", bucket: "films", key: "a.mp4", version_id: null, output: "video/720p/mp4", attempts: 5, error: "ffmpeg ran past its time limit" };
  assert.deepEqual(readFailedRenditions({ failed: [one], more: true }), { failed: [one], more: true });
  assert.deepEqual(readFailedRenditions({ failed: [{ ...one, error: null, version_id: "v1" }], more: false })?.failed[0]?.version_id, "v1");
  assert.equal(readFailedRenditions({ failed: [{ ...one, queue: "audio" }], more: false }), null);
  assert.equal(readFailedRenditions({ failed: [{ ...one, attempts: "5" }], more: false }), null);
  assert.equal(readFailedRenditions({ failed: [one] }), null);
});

test("the sign-in methods say whether a provider is offered, and nothing else is read as one", async () => {
  const { readMethods } = await import("../src/signin-model.ts");
  assert.deepEqual(readMethods({ provider: true, keys: "all" }), { provider: true, keys: "all" });
  assert.deepEqual(readMethods({ provider: true, keys: "root" }), { provider: true, keys: "root" });
  assert.deepEqual(readMethods({ provider: false, keys: "all" }), { provider: false, keys: "all" });
  assert.equal(readMethods({ provider: true, keys: "none" }), null);
  assert.equal(readMethods({ provider: true }), null);
  assert.equal(readMethods({ provider: "yes", keys: "all" }), null);
  assert.equal(readMethods({}), null);
  assert.equal(readMethods(null), null);
});

test("replication is read as its three counts, or as nothing to show", () => {
  const counts = { backlog: 22, in_flight: 3, failed: 0 };
  assert.deepEqual(readStatus({ ...status(null), replication: counts })?.replication, counts);
  assert.equal(readStatus(status(null))?.replication, null);
  assert.equal(readStatus({ ...status(null), replication: { backlog: 1, in_flight: 0 } }), null);
  assert.equal(readStatus({ ...status(null), replication: undefined }), null, "the server always says it, null when there is none");
});

test("an object's detail carries its tags, its retention and its legal hold", () => {
  const detail = { key: "doc.txt", size: 5, etag: '"e"', modified: "2026-10-16T10:00:00.000Z", headers: {}, metadata: {}, checksums: {}, parts: null };
  const kept = { ...detail, tags: { team: "blue" }, retention: { mode: "GOVERNANCE", until: "2026-10-17T10:00:00.000Z" }, legal_hold: true };
  assert.deepEqual(readDetail(kept), {
    ...detail,
    tags: { team: "blue" },
    retention: { mode: "GOVERNANCE", until: "2026-10-17T10:00:00.000Z" },
    legalHold: true,
  });
  assert.deepEqual(readDetail({ ...kept, retention: null, legal_hold: false })?.retention, null, "no retention is null");
  assert.equal(readDetail(detail), null, "the lock and the tags are always sent");
  assert.equal(readDetail({ ...kept, retention: { mode: "FOREVER", until: "2026-10-17T10:00:00.000Z" } }), null, "a mode is GOVERNANCE or COMPLIANCE");
  assert.equal(readDetail({ ...kept, legal_hold: "ON" }), null, "the hold is a boolean");
  assert.equal(readDetail({ ...kept, tags: { team: 1 } }), null, "a tag value is text");
});
