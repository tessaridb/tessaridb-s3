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
export type BucketUsage = { readonly bucket: string; readonly objects: number; readonly bytes: number };
export type Usage = {
  readonly taken: string | null;
  readonly buckets: readonly BucketUsage[];
  readonly objects: number;
  readonly bytes: number;
};
export type Bucket = { readonly name: string; readonly created: string; readonly region: string };
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
  record(v) && text(v["bucket"]) && count(v["objects"]) && count(v["bytes"])
    ? { bucket: v["bucket"], objects: v["objects"], bytes: v["bytes"] }
    : null;

export function readUsage(v: unknown): Usage | null {
  if (!record(v) || !textOrNull(v["taken"]) || !count(v["objects"]) || !count(v["bytes"])) {
    return null;
  }
  const buckets = list(v["buckets"], bucketUsage);
  return buckets === null ? null : { taken: v["taken"], buckets, objects: v["objects"], bytes: v["bytes"] };
}

const bucket = (v: unknown): Bucket | null =>
  record(v) && text(v["name"]) && text(v["created"]) && text(v["region"])
    ? { name: v["name"], created: v["created"], region: v["region"] }
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

export function readProblem(v: unknown): Problem | null {
  return record(v) && text(v["code"]) && text(v["message"]) ? { code: v["code"], message: v["message"] } : null;
}

/** For answers whose body is not read (204, or a 201 whose only content is the name we sent). */
export const ignored = (_v: unknown): true => true;
