"use client";

/**
 * The inbox agent's state, and the status changes it wants to make (Phase 9).
 *
 * # Why the failure line is the important part
 *
 * Rule 5: a broken sync must be visible. Google expires refresh tokens after seven days while
 * the OAuth app is in Testing, so the agent *will* stop — and a stopped agent looks exactly
 * like a quiet job market. The backend already records the outcome and the reason on every
 * run; until this panel existed, "visible" meant a JSON endpoint nobody opens.
 *
 * # Nothing is applied silently
 *
 * Every proposal shows the email that caused it. Rule 2's audit trail is what makes a
 * misclassification reversible, and an audit trail you can only read with SQL is not one.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import {
  decideProposal,
  getInboxStatus,
  listProposals,
  type InboxStatus,
  type StatusProposal,
} from "@/lib/internshipsApi";

function outcomeTone(outcome: string): string {
  if (outcome === "success") return "text-green-700 dark:text-green-400";
  if (outcome === "skipped") return "text-neutral-500";
  return "text-red-600 dark:text-red-400";
}

export function InboxPanel() {
  const handleError = useApiError();
  const [status, setStatus] = useState<InboxStatus | null>(null);
  const [proposals, setProposals] = useState<StatusProposal[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextProposals] = await Promise.all([
        getInboxStatus(),
        listProposals(),
      ]);
      setStatus(nextStatus);
      setProposals(nextProposals);
    } catch (err) {
      setMessage(handleError(err, "Could not load the inbox agent's state"));
    }
  }, [handleError]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [nextStatus, nextProposals] = await Promise.all([
          getInboxStatus(),
          listProposals(),
        ]);
        if (!cancelled) {
          setStatus(nextStatus);
          setProposals(nextProposals);
        }
      } catch (err) {
        if (!cancelled) setMessage(handleError(err, "Could not load the inbox agent's state"));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleError]);

  async function decide(id: string, accept: boolean) {
    setBusy(true);
    setMessage(null);
    try {
      await decideProposal(id, accept);
      await refresh();
      setMessage(accept ? "Applied." : "Left alone.");
    } catch (err) {
      setMessage(handleError(err, "Could not record that decision"));
    } finally {
      setBusy(false);
    }
  }

  if (!status?.account && proposals.length === 0) {
    // Nothing connected and nothing pending: say so in one line rather than rendering an
    // empty panel that looks broken.
    return (
      <section className="rounded border border-neutral-300 p-4 text-sm dark:border-neutral-700">
        <span className="font-semibold">Inbox agent</span>{" "}
        <span className="text-neutral-500">
          — not connected.{" "}
          <a className="underline" href="http://localhost:8080/auth/gmail/start">
            Connect a Gmail account
          </a>
        </span>
      </section>
    );
  }

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <h2 className="font-semibold">Inbox agent</h2>

      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        {status?.account ?? "not connected"}
        {status?.last_run && (
          <>
            {" — last run "}
            <span className={outcomeTone(status.last_run.outcome)}>
              {status.last_run.outcome}
            </span>
            {`, ${status.last_run.classified} classified`}
          </>
        )}
      </p>

      {/* The line rule 5 exists for. A stopped agent must not read as a quiet inbox. */}
      {status?.last_run?.error && (
        <p className="mt-1 rounded border border-red-500/40 bg-red-500/5 px-2 py-1 text-sm text-red-700 dark:text-red-400">
          {status.last_run.error}
        </p>
      )}

      {message && <p className="mt-2 text-sm text-neutral-600">{message}</p>}

      {proposals.length === 0 ? (
        <p className="mt-2 text-sm text-neutral-500">No status changes waiting.</p>
      ) : (
        <ul className="mt-3 space-y-3">
          {proposals.map((proposal) => (
            <li
              key={proposal.id}
              className="border-t border-neutral-200 pt-2 text-sm dark:border-neutral-800"
            >
              <div>
                <span className="font-medium">{proposal.company_name}</span>{" "}
                <span className="text-neutral-500">— {proposal.title}</span>
              </div>
              <div className="mt-0.5">
                <code className="rounded bg-neutral-100 px-1 dark:bg-neutral-900">
                  {proposal.from_status}
                </code>{" "}
                →{" "}
                <code className="rounded bg-neutral-100 px-1 dark:bg-neutral-900">
                  {proposal.to_status}
                </code>
                {proposal.applied_automatically && (
                  <span className="ml-2 text-xs text-amber-700 dark:text-amber-400">
                    already applied — rejecting undoes it
                  </span>
                )}
              </div>

              {/* The email that caused it. Without this the panel asks you to approve a change
                  you have no way to check. */}
              <div className="mt-1 text-xs text-neutral-500">
                {proposal.subject && <div>from: {proposal.subject}</div>}
                {proposal.evidence && <div>matched: {proposal.evidence}</div>}
              </div>

              <div className="mt-1 flex gap-2">
                <button
                  type="button"
                  className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                  onClick={() => decide(proposal.id, true)}
                  disabled={busy}
                >
                  Accept
                </button>
                <button
                  type="button"
                  className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                  onClick={() => decide(proposal.id, false)}
                  disabled={busy}
                >
                  Reject
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
