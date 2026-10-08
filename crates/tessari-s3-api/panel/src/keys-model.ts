//! What the console answers about a user's access keys: an issue (a new key with
//! its secret, or the key the form's token already issued, without one) and a
//! listing, which never carries a secret.

import { count, flag, list, record, text } from "./models.ts";
import type { IssuedKey } from "./models.ts";

/** An issue's answer: the new key, or the one this form already issued. */
export type Issue = { readonly kind: "issued"; readonly key: IssuedKey } | { readonly kind: "already"; readonly access_key_id: string };

/** A one-key credential's reach: one key of one bucket until `expires` (epoch seconds). */
export type UploadReach = { readonly bucket: string; readonly key: string; readonly expires: number };

export type ListedKey = {
  readonly access_key_id: string;
  readonly created: string;
  readonly disabled: boolean;
  readonly upload: UploadReach | null;
};

export function readIssue(v: unknown): Issue | null {
  if (!record(v) || !text(v["access_key_id"])) {
    return null;
  }
  if (text(v["secret_access_key"])) {
    return { kind: "issued", key: { access_key_id: v["access_key_id"], secret_access_key: v["secret_access_key"] } };
  }
  return v["already_issued"] === true ? { kind: "already", access_key_id: v["access_key_id"] } : null;
}

function reach(v: unknown): UploadReach | null | undefined {
  if (v === null) {
    return null;
  }
  return record(v) && text(v["bucket"]) && text(v["key"]) && count(v["expires"]) ? { bucket: v["bucket"], key: v["key"], expires: v["expires"] } : undefined;
}

function listed(v: unknown): ListedKey | null {
  if (!record(v) || !text(v["access_key_id"]) || !text(v["created"]) || !flag(v["disabled"]) || !("upload" in v)) {
    return null;
  }
  const upload = reach(v["upload"]);
  return upload === undefined ? null : { access_key_id: v["access_key_id"], created: v["created"], disabled: v["disabled"], upload };
}

export function readKeys(v: unknown): ListedKey[] | null {
  return record(v) ? list(v["keys"], listed) : null;
}
