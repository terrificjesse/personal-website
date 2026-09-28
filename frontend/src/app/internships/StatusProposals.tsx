"use client";

/**
 * Status changes the mailbox implies, waiting for you to accept or refuse them.
 *
 * # Why this is a component and not a block inside `InboxPanel`
 *
 * The same reason `UntrackedApplications` is one, and the same mistake twice over. The queue
 * lived at the bottom of `/internships`, below the filters, the collection status and the
 * analytics — so the Application outcomes panel could say "47 no response, 1 rejected" while
 * eleven rejections sat one scroll further down, unread. **The review queue has to be where the
 * question it answers is being asked**, which is the applications page.
 *
 * `InboxPanel` still renders it. One component, two mount points, one endpoint — rather than
 * two lists that drift.
 *
 * # Accepting moves a real row
 *
 * Which is why nothing here is automatic. `may_auto_apply` refuses every terminal status at any
 * confidence, so a rejection is a human's decision by construction, not by configuration.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import { UnauthorizedError } from "@/lib/apiClient";
import {
  ProposalStaleError,
  decideProposal,
  listProposals,
  type StatusProposal,
} from "@/lib/internshipsApi";

/** Above this many pending rejections, reviewing them one at a time stops being reading. */
const BULK_THRESHOLD = 3;

export function StatusProposals({
  /** Called after a proposal is accepted, so a surrounding list can pick up the new status. */
  onDecided,
}: {
  onDecided?: () => void;
}) {
  const handleError = useApiError();
  const [items, setItems] = useState<StatusProposal[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /**
   * A load that failed for any reason other than being signed out.
   *
   * This section renders `null` when it holds nothing, which is right for an empty queue and
   * wrong for a broken one — the same trap `UntrackedApplications` documents, where a 500 and
   * "nothing pending" looked identical on screen.
   */
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setItems(await listProposals());
      setLoadError(null);
    } catch (err) {
      setMessage(handleError(err, "Could not load the status changes found in your mail"));
    }
  }, [handleError]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await listProposals();
        if (!cancelled) {
          setItems(loaded);
          setLoadError(null);
        }
      } catch (err) {
        if (cancelled) return;
        // Signed out is the one silent case, and it is silent because it is not a fault.
        if (err instanceof UnauthorizedError) return;
        setLoadError(
          err instanceof Error ? err.message : "Could not load status changes from your mail",
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
      await decideProposal(id, accept);
      await load();
      onDecided?.();
      setMessage(accept ? "Applied to your tracker." : "Left as it was.");
    } catch (err) {
      // Stale is not a failure. The proposal has been settled server-side, so reloading is what
      // makes the queue agree with that.
      if (err instanceof ProposalStaleError) {
        await load();
        onDecided?.();
        setMessage(err.message);
        return;
      }
      setMessage(handleError(err, "Could not record that decision"));
    } finally {
      setBusy(false);
    }
  }

  const rejections = items.filter((item) => item.to_status === "rejected");

  /**
   * Accept every pending rejection.
   *
   * Rejections only: they are the repetitive case, and they are terminal, so there is no
   * pipeline position to get wrong. An OA or interview moves an application *through* stages
   * and gets an individual decision.
   *
   * Sequential rather than `Promise.all`, and it reports where it stopped. A bulk action that
   * half-applied and then said "something went wrong" is indistinguishable from one that did
   * nothing at all, and the difference is eleven rows of your tracker.
   */
  async function acceptAllRejections() {
    setBusy(true);
    setMessage(null);
    let done = 0;
    let skipped = 0;
    try {
      for (const item of rejections) {
        try {
          await decideProposal(item.id, true);
          done += 1;
        } catch (err) {
          // One application having moved is no reason to abandon the other ten. Counted and
          // reported, never swallowed — a bulk action that quietly drops rows is worse than one
          // that stops.
          if (err instanceof ProposalStaleError) {
            skipped += 1;
            continue;
          }
          throw err;
        }
      }
      const tail = skipped > 0 ? `, and skipped ${skipped} that had already moved` : "";
      setMessage(`Applied ${done} rejection${done === 1 ? "" : "s"}${tail}.`);
    } catch (err) {
      const reason = err instanceof Error ? err.message : "unknown error";
      setMessage(`Applied ${done} of ${rejections.length}, then stopped: ${reason}`);
    } finally {
      await load();
      onDecided?.();
      setBusy(false);
    }
  }

  if (items.length === 0) {
    if (!loadError) return null;
    return (
      <section className="mt-6 rounded-lg border border-red-500/40 bg-red-500/5 p-3 text-sm text-red-700 dark:text-red-400">
        Could not load status changes found in your mail: {loadError}
      </section>
    );
  }

  return (
    <section className="mt-6 rounded-lg border border-sky-500/40 bg-sky-500/5 p-4">
      <div className="flex flex-wrap items-baseline gap-x-3">
        <h2 className="font-semibold">
          Status changes from your mail{" "}
          <span className="font-normal text-black/60 dark:text-white/60">({items.length})</span>
        </h2>
        {rejections.length > BULK_THRESHOLD && (
          <button
            type="button"
            className="ml-auto rounded border border-black/20 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-white/25"
            onClick={acceptAllRejections}
            disabled={busy}
          >
            Accept all {rejections.length} rejections
          </button>
        )}
      </div>
      <p className="mt-0.5 text-sm text-black/70 dark:text-white/70">
        Your mail says these applications moved. Nothing is applied until you say so.
      </p>

      {message && <p className="mt-2 text-sm text-black/70 dark:text-white/70">{message}</p>}

      <ul className="mt-3 space-y-3">
        {items.map((item) => (
          <li
            key={item.id}
            className="rounded border border-black/10 bg-white/60 p-3 text-sm dark:border-white/15 dark:bg-black/20"
          >
            <div className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-medium">{item.company_name}</span>
              <span className="text-black/60 dark:text-white/60">{item.title}</span>
              <span className="ml-auto text-xs">
                <code className="rounded bg-black/5 px-1 dark:bg-white/10">
                  {item.from_status}
                </code>{" "}
                →{" "}
                <code className="rounded bg-black/5 px-1 dark:bg-white/10">{item.to_status}</code>
              </span>
            </div>

            {item.applied_automatically && (
              <div className="mt-1 text-xs text-amber-700 dark:text-amber-400">
                already applied — rejecting undoes it
              </div>
            )}

            {/* The email that caused it. Without this the queue asks you to approve a change
                you have no way to check. */}
            {item.evidence_available ? (
              <div className="mt-1 text-xs text-black/55 dark:text-white/55">
                {item.from_address && <div>from: {item.from_address}</div>}
                {item.subject && <div>subject: {item.subject}</div>}
                {item.evidence && <div>matched: {item.evidence}</div>}
              </div>
            ) : (
              <div className="mt-1 text-xs text-amber-700 dark:text-amber-400">
                The email behind this is no longer in the database, so there is nothing to check
                it against. The change itself is still described above.
              </div>
            )}

            <div className="mt-2 flex gap-2">
              <button
                type="button"
                className="rounded border border-black/20 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-white/25"
                onClick={() => decide(item.id, true)}
                disabled={busy}
              >
                Apply it
              </button>
              <button
                type="button"
                className="rounded border border-black/20 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-white/25"
                onClick={() => decide(item.id, false)}
                disabled={busy}
              >
                Not this one
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
