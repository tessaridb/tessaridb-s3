import { test } from "node:test";
import assert from "node:assert/strict";
import { readGroupDetail, readGroups } from "../src/groups-model.ts";

const group = { name: "editors", created: "2026-10-17T10:00:00.000Z" };
const detail = {
  ...group,
  members: ["ann", "bob"],
  grants: [{ bucket: "media", read: true, write: false }],
};

test("a space's groups are read as names with when each was made", () => {
  assert.deepEqual(readGroups({ groups: [group] }), [group]);
  assert.deepEqual(readGroups({ groups: [] }), []);
  assert.equal(readGroups({ groups: [{ name: "editors" }] }), null);
  assert.equal(readGroups([group]), null, "the list is under its own name");
});

test("a group is read with its members and its bucket grants", () => {
  assert.deepEqual(readGroupDetail(detail), detail);
  assert.deepEqual(readGroupDetail({ ...detail, members: [], grants: [] }), { ...detail, members: [], grants: [] });
  assert.equal(readGroupDetail({ ...detail, members: ["ann", 3] }), null);
  assert.equal(readGroupDetail({ ...detail, grants: [{ bucket: "media", read: "yes", write: false }] }), null);
  assert.equal(readGroupDetail({ ...detail, grants: undefined }), null, "the grants are said even when there are none");
});
