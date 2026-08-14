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

/** Signed-in visitors are bounced off these back into the app. */
const AUTH_PATHS = ["/login", "/register"];

export default function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const hasSession = request.cookies.has(SESSION_COOKIE);

  if (AUTH_PATHS.includes(pathname)) {
    if (hasSession) {
      return NextResponse.redirect(new URL("/fridge", request.url));
    }
    return NextResponse.next();
  }

  if (!hasSession) {
    const login = new URL(LOGIN_PATH, request.url);
    // Carry where they were headed, so signing in doesn't dump everyone on the same page.
    // Read back as a *path only* in the login page — see the note there on open redirects.
    login.searchParams.set("next", pathname);
    return NextResponse.redirect(login);
  }

  return NextResponse.next();
}

export const config = {
  // Only the fridge tab is behind auth. The site shell (`/`) stays public — it's a personal
  // project site whose landing page has nothing user-specific on it, and future tabs get to
  // make their own call rather than inheriting the fridge app's.
  //
  // `/login` and `/register` are matched deliberately, not excluded: the redirect-if-signed-in
  // branch above needs to run on them.
  matcher: ["/fridge/:path*", "/login", "/register"],
};
