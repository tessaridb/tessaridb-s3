//! Region failover as the console sees it: each region an operator ever failed
//! over, under which epoch, and where its writes go while it is failed over.

import { count, flag, list, record, text } from "./models.ts";

export type Region = {
  readonly region: string;
  readonly epoch: number;
  readonly failed_over: boolean;
  readonly promoted: string | null;
  readonly reason: string;
  readonly by: string;
  readonly at: string;
};

/** Lowercase letters, digits and single hyphens, beginning with a letter — the names the server takes. */
const REGION = /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/;

export function readRegion(v: unknown): Region | null {
  if (
    !record(v) ||
    !text(v["region"]) ||
    !count(v["epoch"]) ||
    !flag(v["failed_over"]) ||
    !(v["promoted"] === null || text(v["promoted"])) ||
    !text(v["reason"]) ||
    !text(v["by"]) ||
    !text(v["at"])
  ) {
    return null;
  }
  // Failed over exactly when a promoted region is named: anything else is not an answer the server gives.
  if (v["failed_over"] !== (v["promoted"] !== null)) {
    return null;
  }
  return {
    region: v["region"],
    epoch: v["epoch"],
    failed_over: v["failed_over"],
    promoted: v["promoted"],
    reason: v["reason"],
    by: v["by"],
    at: v["at"],
  };
}

export function readRegions(v: unknown): readonly Region[] | null {
  return record(v) ? list(v["regions"], readRegion) : null;
}

/** Whether `typed`, trimmed, is a region name the server takes. */
export function isRegionName(typed: string): boolean {
  return REGION.test(typed.trim());
}
