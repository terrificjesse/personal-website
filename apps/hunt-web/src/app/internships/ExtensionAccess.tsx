"use client";

/**
 * Generate and revoke the tokens the Firefox extension authenticates with.
 *
 * # Why the extension cannot just use the session cookie
 *
 * It was meant to. `fridge_session` is `SameSite=Lax`, and a request from a
 * `moz-extension://` page is cross-site, so Firefox never attaches it — the backend saw an
 * anonymous request and answered 401 while the user was demonstrably signed in here. This
 * panel is the fallback `apps/hunt-extension/CLAUDE.md` names, and it exists only because the
 * cookie was tried first.
 *
 * # The secret is shown exactly once
 *
 * Only its SHA-256 is stored, so it genuinely cannot be shown again — losing it means
 * generating another and revoking this one. The UI has to make that obvious at the moment of
 * creation rather than explaining it afterwards.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import {
  createExtensionToken,
  listExtensionTokens,
  revokeExtensionToken,
  type ExtensionToken,
  type MintedExtensionToken,
} from "@/lib/internshipsApi";

function formatWhen(value: string | null): string {
  if (!value) return "never used";
  return new Date(value).toLocaleDateString();
}

export function ExtensionAccess() {
  const handleError = useApiError();
  const [tokens, setTokens] = useState<ExtensionToken[]>([]);
  const [label, setLabel] = useState("Firefox extension");
  const [minted, setMinted] = useState<MintedExtensionToken | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setTokens(await listExtensionTokens());
    } catch (err) {
      setError(handleError(err, "Could not load tokens"));
    }
  }, [handleError]);

  // The load is awaited before any setState, so nothing is set synchronously in the effect
  // body — the `react-hooks/set-state-in-effect` shape this repo already has two of.
  // `cancelled` covers an unmount mid-flight.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await listExtensionTokens();
        if (!cancelled) setTokens(list);
      } catch (err) {
        if (!cancelled) setError(handleError(err, "Could not load tokens"));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleError]);

  async function generate() {
    setBusy(true);
    setError(null);
    try {
      // Replaces any previously shown secret: two visible at once invites pasting the wrong.
      setMinted(await createExtensionToken(label));
      setCopied(false);
      await refresh();
    } catch (err) {
      setError(handleError(err, "Could not create a token"));
    } finally {
      setBusy(false);
    }
  }

  async function revoke(id: string) {
    setBusy(true);
    try {
      await revokeExtensionToken(id);
      if (minted?.id === id) setMinted(null);
      await refresh();
    } catch (err) {
      setError(handleError(err, "Could not revoke the token"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <h2 className="font-semibold">Extension access</h2>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        The hunt extension can&apos;t use your login cookie — Firefox won&apos;t send it from
        an extension page. Generate a token and paste it into the extension&apos;s Settings.
      </p>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <input
          className="flex-1 rounded border border-neutral-300 px-2 py-1 text-sm dark:border-neutral-700 dark:bg-neutral-900"
          value={label}
          maxLength={100}
          onChange={(event) => setLabel(event.target.value)}
          placeholder="What is this for?"
          aria-label="Token label"
        />
        <button
          type="button"
          className="rounded border border-neutral-300 px-3 py-1 text-sm disabled:opacity-50 dark:border-neutral-700"
          onClick={generate}
          disabled={busy}
        >
          Generate
        </button>
      </div>

      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}

      {minted && (
        <div className="mt-3 rounded border border-amber-400 bg-amber-50 p-3 dark:bg-amber-950">
          <p className="text-sm font-medium">
            Copy this now — it is never shown again.
          </p>
          <code className="mt-2 block break-all rounded bg-white p-2 font-mono text-xs dark:bg-neutral-900">
            {minted.secret}
          </code>
          <button
            type="button"
            className="mt-2 rounded border border-neutral-300 px-2 py-1 text-xs dark:border-neutral-700"
            onClick={async () => {
              await navigator.clipboard.writeText(minted.secret);
              setCopied(true);
            }}
          >
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      )}

      {tokens.length > 0 && (
        <ul className="mt-3 space-y-1 text-sm">
          {tokens.map((token) => (
            <li key={token.id} className="flex items-center justify-between gap-2">
              <span>
                {token.label}{" "}
                <span className="text-neutral-500">— {formatWhen(token.last_used_at)}</span>
              </span>
              <button
                type="button"
                className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                onClick={() => revoke(token.id)}
                disabled={busy}
              >
                Revoke
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
