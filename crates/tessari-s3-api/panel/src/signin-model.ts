//! What the sign-in page is told before anyone signs in: whether this node offers signing in through an identity
//! provider, besides signing in with a key.

/** Which access keys open a console session: any live key, or only the root key as the emergency path. */
export type KeySignIn = "all" | "root";

/** The ways to sign in this node offers. */
export interface Methods {
  readonly provider: boolean;
  readonly keys: KeySignIn;
}

/** `v` as the node's sign-in methods, or null when it is not that shape. */
export function readMethods(v: unknown): Methods | null {
  if (typeof v !== "object" || v === null || !("provider" in v) || !("keys" in v)) {
    return null;
  }
  const provider: unknown = v.provider;
  const keys: unknown = v.keys;
  if (typeof provider !== "boolean" || (keys !== "all" && keys !== "root")) {
    return null;
  }
  return { provider, keys };
}

/** Where a person goes to sign in through the provider: always asking it for a recent authentication, so a fresh
 *  console session is also good for the acts that require one. */
export const PROVIDER_SIGN_IN = "/api/v1/auth/oidc/start?fresh=true";
