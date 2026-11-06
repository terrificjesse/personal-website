import { apiBase, apiFetch } from "./apiClient";

export type AuthenticatedUser = {
  id: string;
  email: string;
  created_at: string;
  is_admin: boolean;
};

/** The backend's error body shape (`routes/auth.rs::ErrorBody`). */
type ErrorBody = { error?: string };

/**
 * Pulls the backend's human-readable message out of a failed response.
 *
 * The backend deliberately returns the *same* message for every failed login — unknown email
 * and wrong password are indistinguishable, so the address can't be probed for existence.
 * Don't add client-side logic that tries to tell them apart; there's nothing there to find.
 */
async function errorMessage(res: Response, fallback: string): Promise<string> {
  try {
    const body: ErrorBody = await res.json();
    return body.error ?? fallback;
  } catch {
    return fallback;
  }
}

export async function register(
  email: string,
  password: string,
): Promise<AuthenticatedUser> {
  const res = await apiFetch("/auth/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not create your account"));
  }
  return res.json();
}

export async function login(
  email: string,
  password: string,
): Promise<AuthenticatedUser> {
  // `apiFetch` throws `UnauthorizedError` on 401, which is the *expected* outcome of a bad
  // password here rather than an "your session expired" signal. Caught and re-thrown with the
  // backend's message so the login form can show something useful.
  const res = await fetch(`${apiBase()}/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
    credentials: "include",
    cache: "no-store",
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Invalid email or password"));
  }
  return res.json();
}

export async function logout(): Promise<void> {
  const res = await apiFetch("/auth/logout", { method: "POST" });
  if (!res.ok) {
    throw new Error("Could not sign out");
  }
}

/**
 * Who's signed in, or `null`. Returns `null` rather than throwing when signed out — that's
 * the ordinary case this endpoint exists to report, not a failure.
 */
export async function fetchMe(): Promise<AuthenticatedUser | null> {
  const res = await fetch(`${apiBase()}/auth/me`, {
    credentials: "include",
    cache: "no-store",
  });

  if (!res.ok) return null;
  return res.json();
}

/**
 * Where to send the browser to connect, or reconnect, a Gmail account for the inbox agent.
 *
 * Through `apiBase()` like every other backend URL. This was a literal
 * `http://localhost:8080/auth/gmail/start` until 2026-11-05, which on a deployed site sent the
 * click to the visitor's own machine.
 *
 * **Must be reached on the same host `GMAIL_REDIRECT_URI` names.** The state cookie is host-scoped
 * and `SameSite=Lax`, so a flow started anywhere else never gets its cookie back and the callback
 * fails its own check. In development that means opening the site at `localhost`, not `127.0.0.1`.
 */
export function gmailConnectUrl(): string {
  return `${apiBase()}/auth/gmail/start`;
}

/**
 * Where to send the browser to start Google sign-in.
 *
 * A full page navigation, not a fetch: the flow is a series of cross-origin redirects through
 * Google's consent screen, which XHR can't follow and CORS wouldn't permit.
 */
export function googleSignInUrl(): string {
  return `${apiBase()}/auth/google/start`;
}
