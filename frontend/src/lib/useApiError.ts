"use client";

import { useCallback } from "react";
import { usePathname, useRouter } from "next/navigation";
import { UnauthorizedError } from "./apiClient";

/**
 * Turns a failed API call into either a redirect to the login page or a message to show.
 *
 * `proxy.ts` catches the common signed-out case before a page ever renders, but it can only
 * check that the cookie *exists*. A session that has expired or been revoked server-side still
 * has a cookie, sails past the proxy, and shows up here as a 401 — which `apiFetch` raises as
 * `UnauthorizedError`. Without this, that lands in the same catch as a dead backend and the
 * user is told to check whether the server is running.
 *
 * Returns the message to display, so the call site stays a one-liner:
 *
 * ```ts
 * .catch((err) => setError(handleApiError(err, "Couldn't reach the fridge API…")))
 * ```
 */
export function useApiError() {
  const router = useRouter();
  const pathname = usePathname();

  return useCallback(
    (error: unknown, fallback: string): string => {
      if (error instanceof UnauthorizedError) {
        router.replace(`/login?next=${encodeURIComponent(pathname)}`);
        return "Your session expired. Redirecting to sign in…";
      }
      return fallback;
    },
    [router, pathname],
  );
}
