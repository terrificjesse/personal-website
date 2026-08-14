/**
 * Shared transport for every call to the Rust backend.
 *
 * Added in Phase 5. Before auth, each `*Api.ts` module held its own copy of `API_BASE` and
 * called `fetch` directly, which was harmless — the requests carried no credentials and there
 * was nothing to get wrong. That stopped being true once a session cookie existed: a single
 * `fetch` that forgets `credentials: "include"` is sent without the cookie and comes back 401,
 * and it looks exactly like being logged out. Routing everything through one function is what
 * makes that impossible rather than merely unlikely.
 *
 * The per-feature modules keep their deliberately separate type names (see
 * `apps/fridge-app/CLAUDE.md`); only the transport is shared.
 */

/** The port the Rust backend binds (see `main.rs`). */
const BACKEND_PORT = 8080;

/**
 * Where the backend lives, derived from whatever host the page was opened on.
 *
 * This used to be a fixed `NEXT_PUBLIC_FRIDGE_API_URL` (pinned to the LAN IP so phones could
 * reach it). That worked fine while requests were anonymous and stops working the moment
 * there's a session cookie, because **cookies are scoped by host and ignore port**:
 *
 * - Page on `localhost:3000` calling an API on `192.168.x.x:8080` is a *cross-site* request,
 *   so a `SameSite=Lax` cookie is never sent at all.
 * - Even if it were, the cookie would belong to host `192.168.x.x`, so `proxy.ts` — which
 *   runs on the `localhost:3000` request — could never see it, and would bounce every
 *   signed-in visit back to the login page.
 *
 * Deriving the host removes the mismatch by construction: open the site at `localhost:3000`
 * and the API is `localhost:8080`; open it at `192.168.x.x:3000` from a phone and the API is
 * `192.168.x.x:8080`. Same host either way, so the cookie works from both with no config.
 *
 * `NEXT_PUBLIC_FRIDGE_API_URL` still overrides, for a deployment where the two genuinely are
 * on different hosts — but that setup needs `SameSite=None; Secure` and therefore HTTPS on
 * both ends. Setting it to a bare LAN IP for local dev is the thing that breaks.
 */
export function apiBase(): string {
  const configured = process.env.NEXT_PUBLIC_FRIDGE_API_URL;
  if (configured) return configured;

  // No `window` during server rendering (`proxy.ts` never calls the backend, so this is only
  // ever a fallback for server-side code that might later want it).
  if (typeof window === "undefined") return `http://127.0.0.1:${BACKEND_PORT}`;

  return `${window.location.protocol}//${window.location.hostname}:${BACKEND_PORT}`;
}

/**
 * Thrown when the backend says the session is missing or expired.
 *
 * A distinct type rather than a generic `Error` so callers can tell "you need to sign in"
 * apart from "that request failed" — the first wants a redirect to the login page, the second
 * wants an error message in place.
 */
export class UnauthorizedError extends Error {
  constructor() {
    super("Not signed in");
    this.name = "UnauthorizedError";
  }
}

/**
 * Calls the backend with the session cookie attached.
 *
 * `credentials` is applied **after** the caller's `init` spread, so it cannot be overridden by
 * accident — the whole point of this function. Note this also requires the backend to send an
 * explicit `Access-Control-Allow-Origin` (never `*`) plus `Allow-Credentials: true`; browsers
 * reject the wildcard on credentialed requests and drop the cookie silently. See
 * `routes/mod.rs::allowed_origins`.
 *
 * `cache: "no-store"` stays the default because every one of these endpoints is per-user,
 * request-time data; a cached fridge is a stale fridge.
 */
export async function apiFetch(
  path: string,
  init: RequestInit = {},
): Promise<Response> {
  const res = await fetch(`${apiBase()}${path}`, {
    cache: "no-store",
    ...init,
    credentials: "include",
  });

  if (res.status === 401) {
    throw new UnauthorizedError();
  }

  return res;
}
