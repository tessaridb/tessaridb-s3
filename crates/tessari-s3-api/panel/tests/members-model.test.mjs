import { test } from "node:test";
import assert from "node:assert/strict";
import { isFingerprint, readMember, readMembers, readToken } from "../src/members-model.ts";

const HEX = "0123456789abcdef".repeat(4);
const member = {
  node: "n7",
  state: "approved",
  fingerprint: HEX,
  awaiting_token: false,
  token_expires: null,
  reason: "seventh node",
  by: "root",
  at: "2026-10-15T10:00:00.000Z",
};

test("a member is read with its state, its pin and whether a token waits", () => {
  assert.deepEqual(readMember(member), member);
  assert.deepEqual(readMember({ ...member, state: "removed", fingerprint: null }), { ...member, state: "removed", fingerprint: null });
  assert.equal(readMember({ ...member, state: "gone" }), null, "only the two states the server writes");
  assert.equal(readMember({ ...member, awaiting_token: "no" }), null);
  assert.equal(readMember({ ...member, fingerprint: undefined }), null, "the pin is said even when there is none");
});

test("the member list is read as a list of members", () => {
  assert.deepEqual(readMembers({ members: [member] }), [member]);
  assert.equal(readMembers({ members: [{ node: "n1" }] }), null);
  assert.equal(readMembers([member]), null);
});

test("an issued token is 64 lowercase hex digits with its lapse", () => {
  const issued = { node: "n8", token: HEX, expires: "2026-10-15T10:10:00.000Z" };
  assert.deepEqual(readToken(issued), issued);
  assert.equal(readToken({ ...issued, token: HEX.toUpperCase() }), null);
  assert.equal(readToken({ ...issued, token: HEX.slice(1) }), null);
  assert.equal(readToken({ node: "n8", token: HEX }), null);
});

test("a fingerprint typed by an operator is 64 hex digits, spaces around it ignored", () => {
  assert.equal(isFingerprint(` ${HEX} `), true);
  assert.equal(isFingerprint(HEX.toUpperCase()), false);
  assert.equal(isFingerprint(HEX.slice(2)), false);
  assert.equal(isFingerprint(""), false);
});
