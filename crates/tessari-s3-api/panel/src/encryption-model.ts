//! This node's encryption keys as the console sees them: not yet created,
//! sealed, or held — and then which key new objects are wrapped under, and
//! the keys operators created by name for `aws:kms`.

import { count, list, record, text } from "./models.ts";

export type Keys =
  | { readonly state: "uninitialised" }
  | { readonly state: "sealed" }
  | {
      readonly state: "unsealed";
      readonly active: string;
      readonly keys: number;
      readonly named: readonly string[];
      readonly behind: readonly Behind[];
    };

/** An older server key this node holds, and how many records still name it. */
export type Behind = { readonly id: string; readonly remaining: number };

function readBehind(v: unknown): Behind | null {
  if (record(v) && text(v["id"]) && count(v["remaining"])) {
    return { id: v["id"], remaining: v["remaining"] };
  }
  return null;
}

export function readKeys(v: unknown): Keys | null {
  if (!record(v)) {
    return null;
  }
  const state = v["state"];
  if (state === "uninitialised" || state === "sealed") {
    return { state };
  }
  const named = list(v["named"], (name) => (text(name) ? name : null));
  const behind = list(v["behind"], readBehind);
  if (state === "unsealed" && text(v["active"]) && count(v["keys"]) && named !== null && behind !== null) {
    return { state, active: v["active"], keys: v["keys"], named, behind };
  }
  return null;
}
