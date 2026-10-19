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
import Link from "next/link";
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
    // **Optional, and presented as optional — decided 2026-10-19.** Most people using the site
    // will never connect an inbox, so this is an offer, not a status: one muted line, no box,
    // no "not connected", which read as something being wrong.
    return (
      <section className="text-sm text-neutral-500 dark:text-neutral-400">
        Optional:{" "}
        <a className="underline" href={GMAIL_CONNECT_URL}>
          connect Gmail
        </a>{" "}
        to track replies to your applications automatically.
        {/* Disconnecting the account does not make already-proposed applications stop
            existing, and they are still worth answering. */}
        <UntrackedApplications onAccepted={refresh} />
      </section>
    );
  }

  return (
    <section className="text-sm">
      <p className="text-neutral-600 dark:text-neutral-400">
        <span className="font-medium">Email sync</span>
        {" · "}
        {status?.account ?? "not connected"}
        {/* Only a run that worked is summarised here. A failed one is the quiet line below,
            and printing "failed" in red up here as well was the loudest thing on the panel. */}
        {status?.last_run && !status.last_run.error && (
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

      {/* The line rule 5 exists for. A stopped agent must not read as a quiet inbox.

          **Quiet since 2026-10-19.** This was a red box. Email sync is optional, and a lapse in
          it is a chore rather than an incident — Google expires the token weekly while the app
          is in Testing — so it is a grey line now. Still visible, still says what to do, and the
          full reason is one click away in the details. */}
      {status?.last_run?.error && !status.last_run.superseded_by_reconnect && (
        <div className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
          Email sync is paused.{" "}
          {/* An error with no way to act on it is half a message.
              
              Google expires refresh tokens after 7 days while the OAuth app is in Testing, so
              this is a scheduled event, not an incident — and the account stays "connected" the
              whole time, which meant the only reconnect link in this panel (the not-connected
              branch below) was never reachable when it was actually needed. Reconnecting is
              always safe: the callback overwrites the stored token, and `prompt=consent` is
              what makes Google re-issue a refresh token rather than silently returning none. */}
          <a className="underline" href={GMAIL_CONNECT_URL}>
            Reconnect
          </a>
          <details className="mt-0.5 text-xs text-neutral-500">
            <summary className="cursor-pointer">Why</summary>
            {status.last_run.error}
          </details>
        </div>
      )}

      {/* Only here, where an inbox is connected: nobody else has mail to label. */}
      <p className="mt-1 text-xs text-neutral-500">
        <Link className="underline" href="/internships/labelling">
          Help measure how accurately your email is sorted
        </Link>
      </p>

      {message && <p className="mt-2 text-sm text-neutral-600">{message}</p>}

      {/* The same queue the applications page shows, and literally the same component: two
          lists over one endpoint would drift, and the drift would be about which changes you
          have already answered. */}
      <StatusProposals onDecided={refresh} />
    </section>
  );
}
