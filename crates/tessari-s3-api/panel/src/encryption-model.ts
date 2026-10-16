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
    };

export function readKeys(v: unknown): Keys | null {
  if (!record(v)) {
    return null;
  }
  const state = v["state"];
  if (state === "uninitialised" || state === "sealed") {
    return { state };
  }
  const named = list(v["named"], (name) => (text(name) ? name : null));
  if (state === "unsealed" && text(v["active"]) && count(v["keys"]) && named !== null) {
    return { state, active: v["active"], keys: v["keys"], named };
  }
  return null;
}
