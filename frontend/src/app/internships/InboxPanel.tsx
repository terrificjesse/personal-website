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
import { StatusProposals } from "./StatusProposals";
import {
  getInboxStatus,
  listProposals,
  type InboxStatus,
  type StatusProposal,
} from "@/lib/internshipsApi";
import { UntrackedApplications } from "./UntrackedApplications";

/// Where the Gmail consent round trip starts.
///
/// **Must be reached on the same host `GMAIL_REDIRECT_URI` names**, which is `localhost` — the
/// state cookie is host-scoped and `SameSite=Lax`, so starting the flow on `127.0.0.1` or a LAN
/// address means the cookie never comes back and the callback fails its own check.
const GMAIL_CONNECT_URL = "http://localhost:8080/auth/gmail/start";

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


  // `proposals` is still fetched here for this one decision: a panel that says "not
  // connected" while changes are waiting would be wrong, and only this component knows
  // whether the account exists. `StatusProposals` below fetches its own and renders nothing
  // when the queue is empty.
  if (!status?.account && proposals.length === 0) {
    // Nothing connected and nothing pending: say so in one line rather than rendering an
    // empty panel that looks broken.
    return (
      <section className="rounded border border-neutral-300 p-4 text-sm dark:border-neutral-700">
        <span className="font-semibold">Inbox agent</span>{" "}
        <span className="text-neutral-500">
          — not connected.{" "}
          <a className="underline" href={GMAIL_CONNECT_URL}>
            Connect a Gmail account
          </a>
        </span>
        {/* Disconnecting the account does not make already-proposed applications stop
            existing, and they are still worth answering. */}
        <UntrackedApplications onAccepted={refresh} />
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
        <div className="mt-1 rounded border border-red-500/40 bg-red-500/5 px-2 py-1 text-sm text-red-700 dark:text-red-400">
          <p>{status.last_run.error}</p>
          {/* An error with no way to act on it is half a message.
              
              Google expires refresh tokens after 7 days while the OAuth app is in Testing, so
              this is a scheduled event, not an incident — and the account stays "connected" the
              whole time, which meant the only reconnect link in this panel (the not-connected
              branch below) was never reachable when it was actually needed. Reconnecting is
              always safe: the callback overwrites the stored token, and `prompt=consent` is
              what makes Google re-issue a refresh token rather than silently returning none. */}
          <a className="mt-0.5 inline-block font-medium underline" href={GMAIL_CONNECT_URL}>
            Reconnect this account
          </a>
        </div>
      )}

      {message && <p className="mt-2 text-sm text-neutral-600">{message}</p>}

      {/* The same queue the applications page shows, and literally the same component: two
          lists over one endpoint would drift, and the drift would be about which changes you
          have already answered. */}
      <StatusProposals onDecided={refresh} />
    </section>
  );
}
