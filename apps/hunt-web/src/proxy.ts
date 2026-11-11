import { NextResponse, type NextRequest } from "next/server";

/**
 * Route protection for the internships site, copied from the personal site's `proxy.ts` when the
 * two split on 2026-11-11. Read that file's notes before changing this one: they explain why
 * this is an optimistic check only, and why `/login` must never be matched.
 *
 * Note the filename: this version of Next renamed the `middleware` convention to `proxy`
 * (see `node_modules/next/dist/docs/01-app/03-api-reference/03-file-conventions/proxy.md`).
 */

/** Must match `auth::SESSION_COOKIE_NAME` in the backend. */
const SESSION_COOKIE = "fridge_session";

/** Signed-out visitors are sent here. */
const LOGIN_PATH = "/login";

export default function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Only the absence of the cookie is acted on: a present cookie may be expired or forged, and
  // only the backend can tell. The backend's `CurrentUser` extractor is the real gate.
  if (request.cookies.has(SESSION_COOKIE)) {
    return NextResponse.next();
  }

  const login = new URL(LOGIN_PATH, request.url);
  login.searchParams.set("next", pathname);
  return NextResponse.redirect(login);
}

export const config = {
  // The listing at `/internships` is public and hides its personal panels when nobody is signed
  // in. The tracker, run health and labelling pages under it 401 without a session, so they are
  // gated. `:path+` needs at least one segment, so it gates the sub-pages and not the listing.
  matcher: ["/internships/:path+"],
};
