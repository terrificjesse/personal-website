"use client";

/**
 * Applications the mailbox implies and the tracker has never heard of.
 *
 * # Why this is a component and not a block inside `InboxPanel`
 *
 * It first shipped inside the inbox panel, which lives at the bottom of `/internships` — below
 * the filters, the collection status and the analytics. The tracker is a **different page**
 * (`/internships/applications`), so someone looking at their applications saw two rows and no
 * indication that eleven more were waiting one route away. The review queue has to be where the
 * question it answers is being asked.
 *
 * # Accepting creates a row
 *
 * Which is why it is a click and not automatic: the classifier is unmeasured until Checkpoint
 * 13, and a wrong application is a row you must notice before you can delete it. "I didn't apply
 * here" is permanent — the same company is never proposed again — so it is labelled as the claim
 * it makes rather than as a generic "Reject".
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import { UnauthorizedError } from "@/lib/apiClient";
import {
  decideUntracked,
  listUntrackedProposals,
  type UntrackedProposal,
} from "@/lib/internshipsApi";

export function UntrackedApplications({
  /** Called after a proposal is accepted, so a surrounding list can pick up the new row. */
  onAccepted,
}: {
  onAccepted?: () => void;
}) {
  const handleError = useApiError();
  const [items, setItems] = useState<UntrackedProposal[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /**
   * A load that failed for any reason other than being signed out.
   *
   * This section renders `null` when it holds nothing, which is right for an empty queue and
   * **wrong for a broken one** — the first version swallowed every first-load error, so a 500
   * and "no proposals" looked identical on screen and one of them meant the feature was dead.
   * That is the quiet-inbox failure `sync`'s rule 7 exists to prevent, reintroduced in the UI.
   */
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setItems(await listUntrackedProposals());
      setLoadError(null);
    } catch (err) {
      setMessage(handleError(err, "Could not check your mail for untracked applications"));
    }
  }, [handleError]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await listUntrackedProposals();
        if (!cancelled) {
          setItems(loaded);
          setLoadError(null);
        }
      } catch (err) {
        if (cancelled) return;
        // Signed out is the one silent case, and it is silent because it is not a fault: a
        // visitor who cannot see the queue should not be told it broke. Everything else is
        // said out loud, because an empty section is otherwise the only symptom.
        if (err instanceof UnauthorizedError) return;
        setLoadError(
          err instanceof Error ? err.message : "Could not load applications found in your mail",
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function decide(id: string, accept: boolean) {
    setBusy(true);
    setMessage(null);
    try {
      await decideUntracked(id, accept);
      await load();
      if (accept) onAccepted?.();
      setMessage(accept ? "Added to your applications." : "Not tracked.");
    } catch (err) {
      setMessage(handleError(err, "Could not record that decision"));
    } finally {
      setBusy(false);
    }
  }

  if (items.length === 0) {
    // Nothing pending renders nothing. A failed load says so instead — see `loadError`.
    if (!loadError) return null;
    return (
      <section className="mt-6 rounded-lg border border-red-500/40 bg-red-500/5 p-3 text-sm text-red-700 dark:text-red-400">
        Could not check your mail for untracked applications: {loadError}
      </section>
    );
  }

  return (
    <section className="mt-6 rounded-lg border border-amber-500/40 bg-amber-500/5 p-4">
      <h2 className="font-semibold">
        Found in your mail{" "}
        <span className="font-normal text-black/60 dark:text-white/60">
          ({items.length})
        </span>
      </h2>
      <p className="mt-0.5 text-sm text-black/70 dark:text-white/70">
        These look like applications you made but never tracked. Nothing is added on its own.
      </p>

      {message && (
        <p className="mt-2 text-sm text-black/70 dark:text-white/70">{message}</p>
      )}

      <ul className="mt-3 space-y-3">
        {items.map((item) => (
          <li
            key={item.id}
            className="rounded border border-black/10 bg-white/60 p-3 text-sm dark:border-white/15 dark:bg-black/20"
          >
            <div className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-medium">{item.company_name}</span>
              {/* `null` means the subject named no role. Say so; the row can be renamed once
                  it exists, and a guess here would be indistinguishable from a fact. */}
              <span className="text-black/60 dark:text-white/60">
                {item.title ?? <em>role not named in the email</em>}
              </span>
              <span className="ml-auto text-xs text-black/60 dark:text-white/60">
                as{" "}
                <code className="rounded bg-black/5 px-1 dark:bg-white/10">
                  {item.implied_status}
                </code>
              </span>
            </div>

            {item.evidence_available ? (
              <div className="mt-1 text-xs text-black/55 dark:text-white/55">
                {item.from_address && <div>from: {item.from_address}</div>}
                {item.subject && <div>subject: {item.subject}</div>}
              </div>
            ) : (
              /* A terse email and a missing one look identical as blank lines, and only one of
                 them means "you cannot check this". */
              <div className="mt-1 text-xs text-amber-700 dark:text-amber-400">
                The email behind this is no longer in the database, so there is nothing to check
                it against.
              </div>
            )}

            <div className="mt-2 flex gap-2">
              <button
                type="button"
                className="rounded border border-black/20 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-white/25"
                onClick={() => decide(item.id, true)}
                disabled={busy}
              >
                Track it
              </button>
              <button
                type="button"
                className="rounded border border-black/20 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-white/25"
                onClick={() => decide(item.id, false)}
                disabled={busy}
              >
                I didn&apos;t apply here
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
