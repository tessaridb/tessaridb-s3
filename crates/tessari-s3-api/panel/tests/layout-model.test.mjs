import { test } from "node:test";
import assert from "node:assert/strict";
import { parseSets, readLayout, readStaged } from "../src/layout-model.ts";

const layout = { version: 2, sets: [[{ node: "n1", zone: "a" }, { node: "n2", zone: null }]] };

test("a staged layout carries its id, its base and the layout it becomes", () => {
  assert.deepEqual(readStaged({ id: "s-1", base: 1, layout }), { id: "s-1", base: 1, layout });
  assert.equal(readStaged({ base: 1, layout }), null, "an apply needs the id it was shown");
  assert.equal(readStaged({ id: "s-1", base: "1", layout }), null);
  assert.equal(readStaged({ id: "s-1", base: 1, layout: { version: 2, sets: [[{ node: "n1" }]] } }), null, "every member says its zone");
});

test("an applied layout is read as a layout", () => {
  assert.deepEqual(readLayout(layout), layout);
  assert.equal(readLayout({ version: 2 }), null);
});

test("sets are typed one per line, nodes split by spaces or commas", () => {
  assert.deepEqual(parseSets("n1 n2, n3\n\n  n4,n5 ,n6  \n"), [["n1", "n2", "n3"], ["n4", "n5", "n6"]]);
  assert.equal(parseSets("n1 n2\n , \n"), null, "a line of separators names no node");
  assert.equal(parseSets("   \n"), null, "no set at all");
});
