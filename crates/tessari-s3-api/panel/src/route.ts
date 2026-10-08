//! The console's views as hash routes, so every view is a URL an operator can
//! reload or hand to a colleague. Keys and prefixes are S3 data and may hold any
//! character, so every value is percent-encoded on the way out and decoded on
//! the way in; a hash that does not name a view is the overview, never an error.

export type Route =
  | { readonly kind: "status" }
  | { readonly kind: "buckets" }
  | { readonly kind: "users" }
  | { readonly kind: "spaces" }
  | { readonly kind: "cluster" }
  | { readonly kind: "objects"; readonly bucket: string; readonly prefix: string; readonly cursor: string | null }
  | { readonly kind: "object"; readonly bucket: string; readonly key: string }
  | { readonly kind: "actions"; readonly before: number | null };

const OVERVIEW: Route = { kind: "status" };

function query(pairs: ReadonlyArray<readonly [string, string | null]>): string {
  const kept = pairs.filter((pair): pair is readonly [string, string] => pair[1] !== null);
  return kept.length === 0
    ? ""
    : `?${kept.map(([name, value]) => `${name}=${encodeURIComponent(value)}`).join("&")}`;
}

/** The hash for `route`. */
export function format(route: Route): string {
  switch (route.kind) {
    case "status":
      return "#/";
    case "buckets":
      return "#/buckets";
    case "users":
      return "#/users";
    case "spaces":
      return "#/spaces";
    case "cluster":
      return "#/cluster";
    case "objects":
      return `#/b/${encodeURIComponent(route.bucket)}${query([
        ["prefix", route.prefix === "" ? null : route.prefix],
        ["cursor", route.cursor],
      ])}`;
    case "object":
      return `#/o/${encodeURIComponent(route.bucket)}${query([["key", route.key]])}`;
    case "actions":
      return `#/actions${query([["before", route.before === null ? null : String(route.before)]])}`;
  }
}

/** A positive whole number, or null. */
function position(text: string | null): number | null {
  if (text === null || !/^[1-9][0-9]*$/.test(text)) {
    return null;
  }
  const value = Number(text);
  return Number.isSafeInteger(value) ? value : null;
}

/** The route `hash` names; the overview when it names none. */
export function parse(hash: string): Route {
  const text = hash.startsWith("#") ? hash.slice(1) : hash;
  const mark = text.indexOf("?");
  const path = mark === -1 ? text : text.slice(0, mark);
  const params = new URLSearchParams(mark === -1 ? "" : text.slice(mark + 1));
  try {
    if (path === "/buckets") {
      return { kind: "buckets" };
    }
    if (path === "/users") {
      return { kind: "users" };
    }
    if (path === "/spaces") {
      return { kind: "spaces" };
    }
    if (path === "/cluster") {
      return { kind: "cluster" };
    }
    if (path === "/actions") {
      return { kind: "actions", before: position(params.get("before")) };
    }
    if (path.startsWith("/b/")) {
      const bucket = decodeURIComponent(path.slice(3));
      return bucket === ""
        ? { kind: "buckets" }
        : { kind: "objects", bucket, prefix: params.get("prefix") ?? "", cursor: params.get("cursor") };
    }
    if (path.startsWith("/o/")) {
      const bucket = decodeURIComponent(path.slice(3));
      const key = params.get("key");
      return bucket === "" || key === null || key === "" ? OVERVIEW : { kind: "object", bucket, key };
    }
  } catch {
    // A malformed percent-escape: the hash names no view.
  }
  return OVERVIEW;
}
