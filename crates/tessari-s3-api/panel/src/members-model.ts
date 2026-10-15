//! Cluster membership as the console sees it: every member and tombstone with
//! its pin and whether a join token waits, the token answer shown once, and the
//! fingerprint an operator types to pin a node to one certificate.

import { flag, list, record, text } from "./models.ts";

export type MemberState = "approved" | "removed";
export type Member = {
  readonly node: string;
  readonly state: MemberState;
  readonly fingerprint: string | null;
  readonly awaiting_token: boolean;
  readonly token_expires: string | null;
  readonly reason: string;
  readonly by: string;
  readonly at: string;
};
/** A join token, shown this once and never again. */
export type IssuedToken = { readonly node: string; readonly token: string; readonly expires: string };

const HEX64 = /^[0-9a-f]{64}$/;
const textOrNull = (value: unknown): value is string | null => value === null || text(value);
const memberState = (value: unknown): value is MemberState => value === "approved" || value === "removed";

export function readMember(v: unknown): Member | null {
  if (
    !record(v) ||
    !text(v["node"]) ||
    !memberState(v["state"]) ||
    !textOrNull(v["fingerprint"]) ||
    !flag(v["awaiting_token"]) ||
    !textOrNull(v["token_expires"]) ||
    !text(v["reason"]) ||
    !text(v["by"]) ||
    !text(v["at"])
  ) {
    return null;
  }
  return {
    node: v["node"],
    state: v["state"],
    fingerprint: v["fingerprint"],
    awaiting_token: v["awaiting_token"],
    token_expires: v["token_expires"],
    reason: v["reason"],
    by: v["by"],
    at: v["at"],
  };
}

export function readMembers(v: unknown): readonly Member[] | null {
  return record(v) ? list(v["members"], readMember) : null;
}

export function readToken(v: unknown): IssuedToken | null {
  if (!record(v) || !text(v["node"]) || !text(v["token"]) || !HEX64.test(v["token"]) || !text(v["expires"])) {
    return null;
  }
  return { node: v["node"], token: v["token"], expires: v["expires"] };
}

/** Whether `typed`, trimmed, is a certificate's SHA-256 as the server takes it. */
export function isFingerprint(typed: string): boolean {
  return HEX64.test(typed.trim());
}
