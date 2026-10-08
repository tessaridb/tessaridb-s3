//! The one way the page talks to the server: same-origin JSON under /api/v1.
//! The session is an HttpOnly cookie the page never sees; a change carries a
//! JSON body, which a cross-site form cannot send. Every answer is checked
//! against its reader before it is used.

import { readProblem } from "./models.ts";

export type Answer<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly status: number; readonly code: string; readonly message: string };

const API = "/api/v1";

/** The path of an API route; every segment the caller passes is already encoded. */
export const path = (route: string): string => `${API}${route}`;

/** `?name=value&…` for the pairs whose value is present, each value percent-encoded. */
export function search(pairs: ReadonlyArray<readonly [string, string | number | null]>): string {
  const kept = pairs.filter((pair): pair is readonly [string, string | number] => pair[1] !== null);
  return kept.length === 0 ? "" : `?${kept.map(([name, value]) => `${name}=${encodeURIComponent(String(value))}`).join("&")}`;
}

/** Calls `method route`, sending `body` as JSON when given, and reads the answer with `read`. */
export async function call<T>(
  method: "GET" | "POST" | "PUT" | "DELETE",
  route: string,
  read: (value: unknown) => T | null,
  body?: Readonly<Record<string, unknown>>,
): Promise<Answer<T>> {
  let response: Response;
  try {
    response = await fetch(path(route), {
      method,
      credentials: "same-origin",
      headers: body === undefined ? { accept: "application/json" } : { accept: "application/json", "content-type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  } catch {
    return { ok: false, status: 0, code: "network", message: "The console did not answer. Check that the node is running, then try again." };
  }
  let parsed: unknown = null;
  if (response.status !== 204) {
    try {
      parsed = await response.json();
    } catch {
      parsed = undefined;
    }
  }
  if (response.ok) {
    const value = parsed === undefined ? null : read(parsed);
    return value === null
      ? { ok: false, status: response.status, code: "bad_answer", message: "The server answered in a shape this page does not understand. Reload the page." }
      : { ok: true, value };
  }
  const problem = readProblem(parsed);
  return {
    ok: false,
    status: response.status,
    code: problem?.code ?? "unknown",
    message: problem?.message ?? `The server refused the request (status ${response.status}).`,
  };
}
