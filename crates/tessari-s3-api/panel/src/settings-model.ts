//! The settings this node's region shares, as the console sees them: a plain
//! setting with its stored value, a secret only as stored or not, each with
//! when a change applies and whether it may change at all.

import { flag, list, record, text } from "./models.ts";

export type Setting = {
  readonly name: string;
  readonly scope: "global" | "region";
  readonly secret: boolean;
  readonly value: string | null;
  readonly stored: boolean;
  readonly live: boolean;
  readonly fixed: boolean;
};

export type Settings = { readonly region: string; readonly settings: readonly Setting[] };

function readSetting(v: unknown): Setting | null {
  if (!record(v) || !text(v["name"]) || !flag(v["stored"]) || !flag(v["fixed"])) {
    return null;
  }
  const scope = v["scope"];
  const kind = v["class"];
  const applies = v["applies"];
  const value = v["value"];
  if ((scope !== "global" && scope !== "region") || (kind !== "setting" && kind !== "secret") || (applies !== "live" && applies !== "start")) {
    return null;
  }
  // A secret never travels with its value; one that does is not an answer this console trusts.
  if ((value !== null && !text(value)) || (kind === "secret" && value !== null)) {
    return null;
  }
  return { name: v["name"], scope, secret: kind === "secret", value, stored: v["stored"], live: applies === "live", fixed: v["fixed"] };
}

export function readSettings(v: unknown): Settings | null {
  if (!record(v) || !text(v["region"])) {
    return null;
  }
  const settings = list(v["settings"], readSetting);
  return settings === null ? null : { region: v["region"], settings };
}
