import { test } from "node:test";
import assert from "node:assert/strict";
import { deleteRefusal } from "../src/object-delete.ts";

const failure = (status, code) => ({ ok: false, status, code, message: "the server's own words" });

test("a delete refused by its ETag says the object changed and was kept", () => {
  const said = deleteRefusal(failure(412, "precondition_failed"));
  assert.equal(said.status, 412);
  assert.match(said.message, /changed since/);
  assert.match(said.message, /not deleted/);
});

test("a delete of an object already gone says so; any other refusal keeps the server's words", () => {
  assert.match(deleteRefusal(failure(404, "no_such_key")).message, /already gone/);
  assert.equal(deleteRefusal(failure(403, "forbidden")).message, "the server's own words");
});
