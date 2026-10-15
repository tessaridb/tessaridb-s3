//! What the sign-in page is told before anyone signs in: whether this node offers signing in through an identity
//! provider, besides signing in with a key.

/** The ways to sign in this node offers. */
export interface Methods {
  readonly provider: boolean;
}

/** `v` as the node's sign-in methods, or null when it is not that shape. */
export function readMethods(v: unknown): Methods | null {
  if (typeof v !== "object" || v === null || !("provider" in v)) {
    return null;
  }
  const provider: unknown = v.provider;
  return typeof provider === "boolean" ? { provider } : null;
}

/** Where a person goes to sign in through the provider: always asking it for a recent authentication, so a fresh
 *  console session is also good for the acts that require one. */
export const PROVIDER_SIGN_IN = "/api/v1/auth/oidc/start?fresh=true";
