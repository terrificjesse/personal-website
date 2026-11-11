"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { fetchMe, logout, type AuthenticatedUser } from "@/lib/authApi";

/**
 * The session corner of the site nav: who's signed in, and a way out.
 *
 * A client component that asks the backend rather than a server component reading the cookie,
 * for a specific reason — the cookie's presence is not the same as the session being valid.
 * Only the backend can tell an expired or revoked token from a live one (`proxy.ts` has the
 * same limitation and says so). Asking `/auth/me` means the header can't claim you're signed
 * in when the backend disagrees.
 */
export function SessionNav() {
  const router = useRouter();
  const pathname = usePathname();
  const [user, setUser] = useState<AuthenticatedUser | null>(null);
  const [loading, setLoading] = useState(true);

  // Re-checked on navigation so signing in or out updates the header without a hard reload.
  useEffect(() => {
    let cancelled = false;

    fetchMe()
      .then((me) => {
        if (!cancelled) setUser(me);
      })
      .catch(() => {
        if (!cancelled) setUser(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [pathname]);

  async function handleLogout() {
    try {
      await logout();
    } finally {
      // Clear locally and navigate even if the request failed — leaving someone looking at a
      // signed-in header after they asked to leave is worse than an optimistic update, and
      // `proxy.ts` will bounce them to /login on the next navigation anyway.
      setUser(null);
      router.replace("/login");
      router.refresh();
    }
  }

  // Renders nothing until the first answer arrives, rather than flashing "Sign in" at someone
  // who is in fact signed in.
  if (loading) return null;

  if (!user) {
    return (
      <Link href="/login" className="ml-auto text-sm opacity-80 hover:opacity-100">
        Sign in
      </Link>
    );
  }

  return (
    <div className="ml-auto flex items-center gap-3">
      <span className="text-sm opacity-60">{user.email}</span>
      <button
        onClick={handleLogout}
        className="text-sm opacity-80 hover:opacity-100 underline underline-offset-4"
      >
        Sign out
      </button>
    </div>
  );
}
