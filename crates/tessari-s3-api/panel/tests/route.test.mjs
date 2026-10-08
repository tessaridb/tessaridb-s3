// Hash routes are the console's shareable URLs: every view must survive being
// written out and read back, whatever a key contains — S3 keys may hold `/`,
// `?`, `&`, `#`, `%`, spaces and any Unicode.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parse, format } from "../src/route.ts";

const hostile = ["a/b/c", "x?y=1&z=2", "frag#ment", "100%", "two  spaces ", "ключ/файл.txt", "+plus", ""];

test("every route survives format then parse", () => {
  const routes = [
    { kind: "status" },
    { kind: "buckets" },
    { kind: "users" },
    { kind: "spaces" },
    { kind: "actions", before: null },
    { kind: "actions", before: 42 },
  ];
  for (const text of hostile) {
    routes.push({ kind: "objects", bucket: "my-bucket", prefix: text, cursor: null });
    routes.push({ kind: "objects", bucket: "my-bucket", prefix: "", cursor: `key:${text}` });
    if (text !== "") routes.push({ kind: "object", bucket: "my.bucket", key: text });
  }
  for (const route of routes) {
    assert.deepEqual(parse(format(route)), route, JSON.stringify(route));
  }
});

test("the formatted hash names the view", () => {
  assert.equal(format({ kind: "status" }), "#/");
  assert.equal(format({ kind: "buckets" }), "#/buckets");
  assert.equal(format({ kind: "object", bucket: "b1b", key: "a b" }), "#/o/b1b?key=a%20b");
});

test("an unknown or malformed hash is the overview, never an error", () => {
  for (const hash of ["", "#", "#/nowhere", "#/o/b1b", "#/actions?before=-3", "#/actions?before=x", "#/b/"]) {
    const route = parse(hash);
    assert.ok(route.kind === "status" || route.kind === "actions" || route.kind === "buckets", hash);
  }
  assert.deepEqual(parse("#/o/b1b"), { kind: "status" });
  assert.deepEqual(parse("#/actions?before=x"), { kind: "actions", before: null });
});

test("users and spaces are their own views", () => {
  assert.equal(format({ kind: "users" }), "#/users");
  assert.equal(format({ kind: "spaces" }), "#/spaces");
  assert.deepEqual(parse("#/users"), { kind: "users" });
  assert.deepEqual(parse("#/spaces"), { kind: "spaces" });
});
