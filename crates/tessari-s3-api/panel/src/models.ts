//! What the console API answers, and the guards that check an answer has that
//! shape before anything reads it. The API is ours, but the page is the boundary
//! where its bytes become values, so nothing is cast — each reader returns the
//! typed value or null.

export type Drive = { readonly capacity: number; readonly free: number; readonly available: number };
export type Member = { readonly node: string; readonly endpoint: string; readonly answering: boolean; readonly drive: Drive | null };
export type Backlog = { readonly listed: number; readonly more: boolean };
export type Status = {
  readonly version: string;
  readonly node: string | null;
  readonly region: string;
  readonly erasure: string | null;
  readonly members: readonly Member[] | null;
  readonly heal_backlog: Backlog | null;
  readonly drive: Drive | null;
};
/** A bucket's figures as last measured: logical bytes, the part of them held inline in the metadata, and the bytes on
 * the drives (with erasure overhead on a cluster). */
export type BucketUsage = {
  readonly bucket: string;
  readonly objects: number;
  readonly bytes: number;
  readonly inline_bytes: number;
  readonly raw_bytes: number;
};
export type Usage = {
  readonly taken: string | null;
  readonly buckets: readonly BucketUsage[];
  readonly objects: number;
  readonly bytes: number;
  readonly inline_bytes: number;
  readonly raw_bytes: number;
};
export type Bucket = {
  readonly name: string;
  readonly created: string;
  readonly region: string;
  /** The most bytes it may hold; null is no limit. */
  readonly maxBytes: number | null;
  /** The most objects it may hold; null is no limit. */
  readonly maxObjects: number | null;
};
export type ObjectRow = { readonly key: string; readonly size: number; readonly etag: string; readonly modified: string };
export type Listing = { readonly objects: readonly ObjectRow[]; readonly prefixes: readonly string[]; readonly next: string | null };
export type Detail = {
  readonly key: string;
  readonly size: number;
  readonly etag: string;
  readonly modified: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly metadata: Readonly<Record<string, string>>;
  readonly checksums: Readonly<Record<string, string>>;
  readonly parts: number | null;
};
export type Action = {
  readonly position: number;
  readonly at: string;
  readonly operator: string;
  readonly operation: string;
  readonly target: string;
  readonly reason: string | null;
  readonly outcome: string;
};
export type Actions = { readonly actions: readonly Action[]; readonly next: number | null };
export type Role = "space_admin" | "member";
export type User = {
  readonly name: string;
  readonly space: string;
  readonly role: Role;
  readonly create_buckets: boolean;
  readonly operator: boolean;
  readonly cluster_viewer: boolean;
  readonly disabled: boolean;
  readonly created: string;
};
export type Space = { readonly name: string; readonly created: string };
/** A key just issued: the only answer that ever carries its secret. */
export type IssuedKey = { readonly access_key_id: string; readonly secret_access_key: string };
/** A one-key upload credential just issued: as a key, and when it stops working. */
export type UploadKey = IssuedKey & { readonly expires: string };
/** What the signed-in key may do, as the server's evaluator answers it. */
export type Capabilities = {
  readonly access_key_id: string;
  readonly operate: boolean;
  readonly administer: boolean;
  readonly view_cluster: boolean;
  readonly issue_upload_keys: boolean;
};
export type Layout = { readonly version: number; readonly nodes: readonly string[] };
export type Cluster = {
  readonly node: string | null;
  readonly erasure: string | null;
  readonly members: readonly Member[] | null;
  readonly layout: Layout | null;
  readonly heal_backlog: Backlog;
  readonly metadata: { readonly addresses: readonly string[] };
};
export type Problem = { readonly code: string; readonly message: string };

type Fields = Readonly<Record<string, unknown>>;

const record = (value: unknown): value is Fields =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const count = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const textOrNull = (value: unknown): value is string | null => value === null || text(value);
const countOrNull = (value: unknown): value is number | null => value === null || count(value);

function list<T>(value: unknown, item: (entry: unknown) => T | null): T[] | null {
  if (!Array.isArray(value)) {
    return null;
  }
  const items: T[] = [];
  for (const entry of value) {
    const read = item(entry);
    if (read === null) {
      return null;
    }
    items.push(read);
  }
  return items;
}

function strings(value: unknown): Record<string, string> | null {
  if (!record(value)) {
    return null;
  }
  const out: Record<string, string> = {};
  for (const [name, entry] of Object.entries(value)) {
    if (!text(entry)) {
      return null;
    }
    out[name] = entry;
  }
  return out;
}

/** A drive, null for an explicit null, and undefined for anything else. */
function drive(v: unknown): Drive | null | undefined {
  if (v === null) {
    return null;
  }
  return record(v) && count(v["capacity"]) && count(v["free"]) && count(v["available"])
    ? { capacity: v["capacity"], free: v["free"], available: v["available"] }
    : undefined;
}

function member(v: unknown): Member | null {
  if (!record(v) || !text(v["node"]) || !text(v["endpoint"]) || typeof v["answering"] !== "boolean") {
    return null;
  }
  const space = drive(v["drive"]);
  return space === undefined ? null : { node: v["node"], endpoint: v["endpoint"], answering: v["answering"], drive: space };
}

export function readStatus(v: unknown): Status | null {
  if (!record(v) || !text(v["version"]) || !textOrNull(v["node"]) || !text(v["region"]) || !textOrNull(v["erasure"])) {
    return null;
  }
  const members = v["members"] === null ? null : list(v["members"], member);
  const backlog = v["heal_backlog"];
  const heal =
    backlog === null ? null : record(backlog) && count(backlog["listed"]) && typeof backlog["more"] === "boolean"
      ? { listed: backlog["listed"], more: backlog["more"] }
      : undefined;
  const space = drive(v["drive"]);
  if ((v["members"] !== null && members === null) || heal === undefined || space === undefined) {
    return null;
  }
  return { version: v["version"], node: v["node"], region: v["region"], erasure: v["erasure"], members, heal_backlog: heal, drive: space };
}

const bucketUsage = (v: unknown): BucketUsage | null =>
  record(v) && text(v["bucket"]) && count(v["objects"]) && count(v["bytes"]) && count(v["inline_bytes"]) && count(v["raw_bytes"])
    ? { bucket: v["bucket"], objects: v["objects"], bytes: v["bytes"], inline_bytes: v["inline_bytes"], raw_bytes: v["raw_bytes"] }
    : null;

export function readUsage(v: unknown): Usage | null {
  if (!record(v) || !textOrNull(v["taken"]) || !count(v["objects"]) || !count(v["bytes"]) || !count(v["inline_bytes"]) || !count(v["raw_bytes"])) {
    return null;
  }
  const buckets = list(v["buckets"], bucketUsage);
  return buckets === null
    ? null
    : { taken: v["taken"], buckets, objects: v["objects"], bytes: v["bytes"], inline_bytes: v["inline_bytes"], raw_bytes: v["raw_bytes"] };
}

const bucket = (v: unknown): Bucket | null =>
  record(v) && text(v["name"]) && text(v["created"]) && text(v["region"]) && countOrNull(v["max_bytes"]) && countOrNull(v["max_objects"])
    ? { name: v["name"], created: v["created"], region: v["region"], maxBytes: v["max_bytes"], maxObjects: v["max_objects"] }
    : null;

export function readBuckets(v: unknown): readonly Bucket[] | null {
  return record(v) ? list(v["buckets"], bucket) : null;
}

const objectRow = (v: unknown): ObjectRow | null =>
  record(v) && text(v["key"]) && count(v["size"]) && text(v["etag"]) && text(v["modified"])
    ? { key: v["key"], size: v["size"], etag: v["etag"], modified: v["modified"] }
    : null;

export function readListing(v: unknown): Listing | null {
  if (!record(v) || !textOrNull(v["next"])) {
    return null;
  }
  const objects = list(v["objects"], objectRow);
  const prefixes = list(v["prefixes"], (entry) => (text(entry) ? entry : null));
  return objects === null || prefixes === null ? null : { objects, prefixes, next: v["next"] };
}

export function readDetail(v: unknown): Detail | null {
  if (!record(v) || !text(v["key"]) || !count(v["size"]) || !text(v["etag"]) || !text(v["modified"]) || !countOrNull(v["parts"])) {
    return null;
  }
  const headers = strings(v["headers"]);
  const metadata = strings(v["metadata"]);
  const checksums = strings(v["checksums"]);
  if (headers === null || metadata === null || checksums === null) {
    return null;
  }
  return { key: v["key"], size: v["size"], etag: v["etag"], modified: v["modified"], headers, metadata, checksums, parts: v["parts"] };
}

const action = (v: unknown): Action | null =>
  record(v) && count(v["position"]) && text(v["at"]) && text(v["operator"]) && text(v["operation"]) &&
  text(v["target"]) && textOrNull(v["reason"]) && text(v["outcome"])
    ? {
        position: v["position"],
        at: v["at"],
        operator: v["operator"],
        operation: v["operation"],
        target: v["target"],
        reason: v["reason"],
        outcome: v["outcome"],
      }
    : null;

export function readActions(v: unknown): Actions | null {
  if (!record(v) || !countOrNull(v["next"])) {
    return null;
  }
  const actions = list(v["actions"], action);
  return actions === null ? null : { actions, next: v["next"] };
}

const flag = (value: unknown): value is boolean => typeof value === "boolean";
const role = (value: unknown): value is Role => value === "space_admin" || value === "member";

const user = (v: unknown): User | null =>
  record(v) && text(v["name"]) && text(v["space"]) && role(v["role"]) && flag(v["create_buckets"]) &&
  flag(v["operator"]) && flag(v["cluster_viewer"]) && flag(v["disabled"]) && text(v["created"])
    ? {
        name: v["name"],
        space: v["space"],
        role: v["role"],
        create_buckets: v["create_buckets"],
        operator: v["operator"],
        cluster_viewer: v["cluster_viewer"],
        disabled: v["disabled"],
        created: v["created"],
      }
    : null;

export function readUsers(v: unknown): readonly User[] | null {
  return record(v) ? list(v["users"], user) : null;
}

const space = (v: unknown): Space | null =>
  record(v) && text(v["name"]) && text(v["created"]) ? { name: v["name"], created: v["created"] } : null;

export function readSpaces(v: unknown): readonly Space[] | null {
  return record(v) ? list(v["spaces"], space) : null;
}

export function readIssued(v: unknown): IssuedKey | null {
  return record(v) && text(v["access_key_id"]) && text(v["secret_access_key"])
    ? { access_key_id: v["access_key_id"], secret_access_key: v["secret_access_key"] }
    : null;
}

export function readUploadKey(v: unknown): UploadKey | null {
  const key = readIssued(v);
  return key !== null && record(v) && text(v["expires"]) ? { ...key, expires: v["expires"] } : null;
}

export function readCapabilities(v: unknown): Capabilities | null {
  return record(v) && text(v["access_key_id"]) && flag(v["operate"]) && flag(v["administer"]) && flag(v["view_cluster"]) && flag(v["issue_upload_keys"])
    ? {
        access_key_id: v["access_key_id"],
        operate: v["operate"],
        administer: v["administer"],
        view_cluster: v["view_cluster"],
        issue_upload_keys: v["issue_upload_keys"],
      }
    : null;
}

const names = (value: unknown): string[] | null => list(value, (entry) => (text(entry) ? entry : null));

function layout(v: unknown): Layout | null | undefined {
  if (v === null) {
    return null;
  }
  const nodes = record(v) && count(v["version"]) ? names(v["nodes"]) : null;
  return record(v) && count(v["version"]) && nodes !== null ? { version: v["version"], nodes } : undefined;
}

export function readCluster(v: unknown): Cluster | null {
  if (!record(v) || !textOrNull(v["node"]) || !textOrNull(v["erasure"]) || !record(v["metadata"])) {
    return null;
  }
  const members = v["members"] === null ? null : list(v["members"], member);
  const placed = layout(v["layout"]);
  const backlog = v["heal_backlog"];
  const addresses = names(v["metadata"]["addresses"]);
  if ((v["members"] !== null && members === null) || placed === undefined || addresses === null) {
    return null;
  }
  if (!record(backlog) || !count(backlog["listed"]) || !flag(backlog["more"])) {
    return null;
  }
  return {
    node: v["node"],
    erasure: v["erasure"],
    members,
    layout: placed,
    heal_backlog: { listed: backlog["listed"], more: backlog["more"] },
    metadata: { addresses },
  };
}

export function readProblem(v: unknown): Problem | null {
  return record(v) && text(v["code"]) && text(v["message"]) ? { code: v["code"], message: v["message"] } : null;
}

/** For answers whose body is not read (204, or a 201 whose only content is the name we sent). */
export const ignored = (_v: unknown): true => true;
