"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { googleSignInUrl, login, register } from "@/lib/authApi";

/** Mirrors `auth::MIN_PASSWORD_LENGTH`. The backend enforces it; this just avoids a round
 *  trip to be told something the form already knows. */
const MIN_PASSWORD_LENGTH = 12;

type Mode = "login" | "register";

/**
 * Shared sign-in / sign-up form. One component because the two differ only in which endpoint
 * they call and what the copy says — two near-identical files would drift.
 */
export function CredentialsForm({ mode }: { mode: Mode }) {
  const router = useRouter();
  const searchParams = useSearchParams();

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const isRegister = mode === "register";

  /**
   * Where to go after signing in. `proxy.ts` puts the originally requested path here.
   *
   * Only ever used if it's a *relative path on this site*. A `next` value is attacker-supplied
   * — it arrives in a URL anyone can construct and send — so accepting `https://evil.example`
   * would turn this form into an open redirect that borrows the site's credibility for a
   * phishing page. The `//` check matters too: `//evil.example` is protocol-relative and
   * navigates off-site despite starting with a slash.
   */
  function destination(): string {
    const next = searchParams.get("next");
    if (next && next.startsWith("/") && !next.startsWith("//")) {
      return next;
    }
    return "/fridge";
  }

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setError(null);
    setSubmitting(true);

    try {
      if (isRegister) {
        await register(email, password);
      } else {
        await login(email, password);
      }
      // `refresh()` re-runs the server render so the nav picks up the new session; without it
      // the header would keep showing "Sign in" until a hard reload.
      router.replace(destination());
      router.refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong");
      setSubmitting(false);
    }
  }

  return (
    <div className="mx-auto max-w-sm px-4 py-16">
      <h1 className="text-xl font-semibold">
        {isRegister ? "Create an account" : "Sign in"}
      </h1>
      <p className="mt-1 text-sm opacity-70">
        {isRegister
          ? "You'll need one to use the fridge app."
          : "Welcome back."}
      </p>

      <form onSubmit={handleSubmit} className="mt-8 flex flex-col gap-4">
        <label className="flex flex-col gap-1">
          <span className="text-sm">Email</span>
          <input
            type="email"
            required
            autoComplete="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="rounded border border-black/15 dark:border-white/20 bg-transparent px-3 py-2 text-sm"
          />
        </label>

        <label className="flex flex-col gap-1">
          <span className="text-sm">Password</span>
          <input
            type="password"
            required
            minLength={isRegister ? MIN_PASSWORD_LENGTH : undefined}
            // Tells a password manager which flow this is, so it offers to generate a new
            // password on sign-up and autofill an existing one on sign-in.
            autoComplete={isRegister ? "new-password" : "current-password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="rounded border border-black/15 dark:border-white/20 bg-transparent px-3 py-2 text-sm"
          />
          {isRegister && (
            <span className="text-xs opacity-60">
              At least {MIN_PASSWORD_LENGTH} characters.
            </span>
          )}
        </label>

        {error && <p className="text-sm text-red-600">{error}</p>}

        <button
          type="submit"
          disabled={submitting}
          className="rounded bg-foreground text-background px-3 py-2 text-sm font-medium disabled:opacity-50"
        >
          {submitting ? "…" : isRegister ? "Create account" : "Sign in"}
        </button>
      </form>

      <div className="mt-6 border-t border-black/10 dark:border-white/10 pt-6">
        {/* A full page navigation, not a fetch: the flow is a chain of cross-origin redirects
            through Google's consent screen, which a fetch can neither follow nor be allowed
            to.

            Navigating on click rather than rendering `href={googleSignInUrl()}` on an <a>,
            because that URL is host-dependent: `apiBase()` derives it from
            `window.location`, which doesn't exist during server rendering. The server would
            emit the 127.0.0.1 fallback, the client would produce localhost, and React would
            report a hydration mismatch on the href. Computing it inside the handler means
            nothing host-dependent is ever rendered, so there's nothing to mismatch. */}
        <button
          type="button"
          onClick={() => {
            window.location.href = googleSignInUrl();
          }}
          className="block w-full rounded border border-black/15 dark:border-white/20 px-3 py-2 text-center text-sm"
        >
          Continue with Google
        </button>
      </div>

      <p className="mt-6 text-sm opacity-70">
        {isRegister ? "Already have an account? " : "Need an account? "}
        <Link
          href={isRegister ? "/login" : "/register"}
          className="underline underline-offset-4"
        >
          {isRegister ? "Sign in" : "Create one"}
        </Link>
      </p>
    </div>
  );
}
