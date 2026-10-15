import { test } from "node:test";
import assert from "node:assert/strict";
import { isRegionName, readRegion, readRegions } from "../src/regions-model.ts";

const region = {
  region: "us-east-1",
  epoch: 1,
  failed_over: true,
  promoted: "eu-west-1",
  reason: "drill",
  by: "root",
  at: "2026-10-16T05:00:00.000Z",
};

test("a region's failover record is read with its epoch and where its writes go", () => {
  assert.deepEqual(readRegion(region), region);
  const restored = { ...region, epoch: 2, failed_over: false, promoted: null };
  assert.deepEqual(readRegion(restored), restored);
  assert.equal(readRegion({ ...region, epoch: -1 }), null);
  assert.equal(readRegion({ ...region, promoted: undefined }), null, "the promoted region is said even when there is none");
  assert.equal(readRegion({ ...region, failed_over: false }), null, "failed over exactly when a promoted region is named");
});

test("the region list is read as a list of records", () => {
  assert.deepEqual(readRegions({ regions: [region] }), [region]);
  assert.deepEqual(readRegions({ regions: [] }), []);
  assert.equal(readRegions({ regions: [{ ...region, by: 1 }] }), null);
  assert.equal(readRegions([region]), null);
});

test("a region name is what the server takes", () => {
  assert.ok(isRegionName("eu-west-1"));
  assert.ok(!isRegionName("EU-west-1"));
  assert.ok(!isRegionName("1eu"));
  assert.ok(!isRegionName("eu--west"));
  assert.ok(!isRegionName(""));
});
