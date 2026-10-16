//! This node's encryption keys as the console sees them: not yet created,
//! sealed, or held — and then which key new objects are wrapped under.

import { count, record, text } from "./models.ts";

export type Keys =
  | { readonly state: "uninitialised" }
  | { readonly state: "sealed" }
  | { readonly state: "unsealed"; readonly active: string; readonly keys: number };

export function readKeys(v: unknown): Keys | null {
  if (!record(v)) {
    return null;
  }
  const state = v["state"];
  if (state === "uninitialised" || state === "sealed") {
    return { state };
  }
  if (state === "unsealed" && text(v["active"]) && count(v["keys"])) {
    return { state, active: v["active"], keys: v["keys"] };
  }
  return null;
}
