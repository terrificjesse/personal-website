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
 * Sender and subject are rendered as separate fields: either may be absent, and the subject
 * must never stand in for the sender.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import {
  decideProposal,
  decideUntracked,
  getInboxStatus,
  listProposals,
  listUntrackedProposals,
  type InboxStatus,
  type StatusProposal,
  type UntrackedProposal,
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
  const [untracked, setUntracked] = useState<UntrackedProposal[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextProposals, nextUntracked] = await Promise.all([
        getInboxStatus(),
        listProposals(),
        listUntrackedProposals(),
      ]);
      setStatus(nextStatus);
      setProposals(nextProposals);
      setUntracked(nextUntracked);
    } catch (err) {
      setMessage(handleError(err, "Could not load the inbox agent's state"));
    }
  }, [handleError]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [nextStatus, nextProposals, nextUntracked] = await Promise.all([
          getInboxStatus(),
          listProposals(),
          listUntrackedProposals(),
        ]);
        if (!cancelled) {
          setStatus(nextStatus);
          setProposals(nextProposals);
          setUntracked(nextUntracked);
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

  async function decideUntrackedProposal(id: string, accept: boolean) {
    setBusy(true);
    setMessage(null);
    try {
      await decideUntracked(id, accept);
      await refresh();
      // Says what actually happened. "Tracked." would be vague about which of the two
      // outcomes occurred, and one of them created a row in the tracker.
      setMessage(accept ? "Added to your applications." : "Not tracked.");
    } catch (err) {
      setMessage(handleError(err, "Could not record that decision"));
    } finally {
      setBusy(false);
    }
  }

  if (!status?.account && proposals.length === 0 && untracked.length === 0) {
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

      {/* Reconnected since the failure: say so instead of showing a reason that no longer
          applies. Left unhandled, the panel insists the token is dead for up to fifteen
          minutes after you replaced it. */}
      {status?.last_run?.superseded_by_reconnect && status.last_run.error && (
        <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
          Reconnected since the last run — it will sync within 15 minutes.
        </p>
      )}

      {/* The line rule 5 exists for. A stopped agent must not read as a quiet inbox. */}
      {status?.last_run?.error && !status.last_run.superseded_by_reconnect && (
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
              {proposal.evidence_available ? (
                <div className="mt-1 text-xs text-neutral-500">
                  {proposal.from_address && <div>from: {proposal.from_address}</div>}
                  {proposal.subject && <div>subject: {proposal.subject}</div>}
                  {proposal.evidence && <div>matched: {proposal.evidence}</div>}
                </div>
              ) : (
                /* Said outright rather than left as three missing lines. A terse email and a
                   missing one look identical otherwise, and only one of them means "you cannot
                   check this". Both buttons stay enabled: the proposal itself is intact, and
                   refusing to let it be accepted would decide for the reader that it is wrong,
                   which is not something this panel knows. */
                <div className="mt-1 text-xs text-amber-700 dark:text-amber-400">
                  The email behind this proposal is no longer in the database, so there is
                  nothing to check it against. The change itself is still described above.
                </div>
              )}

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

      {/* Applications the mailbox implies and the tracker has never heard of.
          
          Its own section rather than mixed into the list above, because the question is
          different: those ask "should this application move?", these ask "does this application
          exist?". Accepting here CREATES a row, which is a heavier action than moving one, and
          burying it among status changes would make the two buttons look interchangeable. */}
      {untracked.length > 0 && (
        <div className="mt-4 border-t border-neutral-200 pt-3 dark:border-neutral-800">
          <h3 className="text-sm font-semibold">
            Applications found in your mail{" "}
            <span className="font-normal text-neutral-500">({untracked.length})</span>
          </h3>
          <p className="mt-0.5 text-xs text-neutral-500">
            These emails look like applications you made but never tracked. Accepting adds them
            to your list; nothing is added on its own.
          </p>

          <ul className="mt-3 space-y-3">
            {untracked.map((item) => (
              <li
                key={item.id}
                className="border-t border-neutral-200 pt-2 text-sm dark:border-neutral-800"
              >
                <div>
                  <span className="font-medium">{item.company_name}</span>{" "}
                  {/* `null` means the subject named no role. Saying so beats inventing one,
                      and the row can be renamed after it is created. */}
                  <span className="text-neutral-500">
                    — {item.title ?? <em>role not named in the email</em>}
                  </span>
                </div>
                <div className="mt-0.5 text-neutral-500">
                  would be added as{" "}
                  <code className="rounded bg-neutral-100 px-1 dark:bg-neutral-900">
                    {item.implied_status}
                  </code>
                </div>

                {item.evidence_available ? (
                  <div className="mt-1 text-xs text-neutral-500">
                    {item.from_address && <div>from: {item.from_address}</div>}
                    {item.subject && <div>subject: {item.subject}</div>}
                    {item.evidence && <div>matched: {item.evidence}</div>}
                  </div>
                ) : (
                  <div className="mt-1 text-xs text-amber-700 dark:text-amber-400">
                    The email behind this is no longer in the database, so there is nothing to
                    check it against.
                  </div>
                )}

                <div className="mt-1 flex gap-2">
                  <button
                    type="button"
                    className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                    onClick={() => decideUntrackedProposal(item.id, true)}
                    disabled={busy}
                  >
                    Track it
                  </button>
                  <button
                    type="button"
                    className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                    onClick={() => decideUntrackedProposal(item.id, false)}
                    disabled={busy}
                  >
                    {/* Not "Reject": this is a claim about whether you applied, and rejecting
                        is permanent — the same company is never asked about again. */}
                    I didn&apos;t apply here
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
