import { NextResponse, type NextRequest } from "next/server";

/**
 * Route protection for the fridge tab.
 *
 * Note the filename: this version of Next renamed the `middleware` convention to `proxy`
 * (see `node_modules/next/dist/docs/01-app/03-api-reference/03-file-conventions/proxy.md`).
 * A `middleware.ts` here would be silently ignored.
 *
 * ## This is an optimistic check, and only an optimistic check
 *
 * All it does is look for the presence of the session cookie. It cannot tell whether the
 * token is valid, unexpired, or belongs to anyone — only the Rust backend can, since only the
 * backend can hash it and hit the `sessions` table. So this redirects people who obviously
 * aren't signed in, and nothing more.
 *
 * The real enforcement is the `CurrentUser` extractor on every backend route, which 401s
 * regardless of what happens here. That ordering matters: if this file were the only gate, a
 * forged or expired cookie would sail past it. Next's own auth guide makes the same point —
 * keep the security check next to the data, and treat this layer as UX.
 *
 * Reading the cookie works because the backend sets it host-only on `localhost` (or the LAN
 * IP), and cookies ignore port — so a cookie set by `:8080` is visible to `:3000`. That's the
 * same property that makes `SameSite=Lax` sufficient; see `routes/auth.rs`'s module doc.
 */

/** Must match `auth::SESSION_COOKIE_NAME` in the backend. */
const SESSION_COOKIE = "fridge_session";

/** Signed-out visitors are sent here. */
const LOGIN_PATH = "/login";

/**
 * Only the *absence* of the cookie is acted on, and that asymmetry is deliberate.
 *
 * No cookie means definitively not signed in, so redirecting to the login page is safe. A
 * cookie being *present* means nothing — it may be expired, revoked, or forged, and only the
 * backend can tell.
 *
 * This file used to also bounce visitors *away* from `/login` when a cookie was present, and
 * that produced a lockout: once a session expired the cookie stayed in the browser, so
 * `/login` redirected to `/fridge`, `/fridge` loaded and got a 401 from the API, `useApiError`
 * redirected back to `/login`, and round it went. The login page became unreachable exactly
 * when it was needed most.
 *
 * If the "you're already signed in, go to the app" bounce is ever wanted back, it has to live
 * somewhere that can check the session is *real* — a client component calling `/auth/me`, the
 * way `SessionNav` does. Never here.
 */
export default function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;

  if (request.cookies.has(SESSION_COOKIE)) {
    return NextResponse.next();
  }

  const login = new URL(LOGIN_PATH, request.url);
  // Carry where they were headed, so signing in doesn't dump everyone on the same page.
  // Read back as a *path only* in the login page — see the note there on open redirects.
  login.searchParams.set("next", pathname);
  return NextResponse.redirect(login);
}

export const config = {
  // Only the fridge tab is behind auth. The site shell (`/`) stays public — it's a personal
  // project site whose landing page has nothing user-specific on it, and future tabs get to
  // make their own call rather than inheriting the fridge app's.
  //
  // `/login` and `/register` are deliberately **not** matched. They must always render, or a
  // stale cookie locks the user out of the only page that could fix it.
  matcher: ["/fridge/:path*"],
};
