//! A space's groups as the console sees them: each group's name and when it was
//! made, and for one group its members and the buckets it is granted, with
//! whether each grant reads and writes.

import { flag, list, record, text } from "./models.ts";

export type Group = { readonly name: string; readonly created: string };
export type GroupGrant = { readonly bucket: string; readonly read: boolean; readonly write: boolean };
export type GroupDetail = Group & { readonly members: readonly string[]; readonly grants: readonly GroupGrant[] };

const group = (v: unknown): Group | null =>
  record(v) && text(v["name"]) && text(v["created"]) ? { name: v["name"], created: v["created"] } : null;

const grant = (v: unknown): GroupGrant | null =>
  record(v) && text(v["bucket"]) && flag(v["read"]) && flag(v["write"]) ? { bucket: v["bucket"], read: v["read"], write: v["write"] } : null;

export function readGroups(v: unknown): readonly Group[] | null {
  return record(v) ? list(v["groups"], group) : null;
}

export function readGroupDetail(v: unknown): GroupDetail | null {
  const named = group(v);
  if (named === null || !record(v)) {
    return null;
  }
  const members = list(v["members"], (entry) => (text(entry) ? entry : null));
  const grants = list(v["grants"], grant);
  return members === null || grants === null ? null : { ...named, members, grants };
}
