"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useApiError } from "@/lib/useApiError";
import {
  APPLICATION_STATUSES,
  APPLICATION_STATUS_LABELS,
  deleteApplication,
  formatDate,
  listApplications,
  updateApplication,
  type ApplicationStatus,
  type InternshipApplication,
} from "@/lib/internshipsApi";

/**
 * The applied tracker.
 *
 * # This page renders entirely from the application's own snapshot
 *
 * Company, title, URL, pay and term were all copied onto the application row when you pressed
 * "I applied", precisely so this view keeps working after the posting closes, is expired by
 * the sweep, or is deleted outright. **Do not make any field here conditional on
 * `posting_is_live`** — that would reintroduce the exact data-loss this design prevents.
 *
 * `posting_is_live` is three-valued and only ever decorates: `true` open, `false` closed, and
 * `null` meaning *we can't tell* — either it was never linked, or the row is gone. `null` must
 * never render as "Closed"; we don't know that, and the snapshot is still good.
 */
export default function ApplicationsPage() {
  const handleApiError = useApiError();
  const [applications, setApplications] = useState<InternshipApplication[] | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  // Fetch inside the effect, so nothing sets state synchronously as the effect runs.
  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const loaded = await listApplications();
        if (cancelled) return;
        setApplications(loaded);
        setError(null);
      } catch (err) {
        if (cancelled) return;
        setError(handleApiError(err, "Couldn't load your applications."));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [handleApiError]);

  const changeStatus = async (id: string, status: ApplicationStatus) => {
    setBusyId(id);
    try {
      const updated = await updateApplication(id, { status });
      setApplications((current) =>
        (current ?? []).map((app) => (app.id === id ? updated : app)),
      );
      setError(null);
    } catch (err) {
      setError(handleApiError(err, "Couldn't update that application."));
    } finally {
      setBusyId(null);
    }
  };

  const remove = async (id: string) => {
    setBusyId(id);
    try {
      await deleteApplication(id);
      setApplications((current) => (current ?? []).filter((a) => a.id !== id));
      setError(null);
    } catch (err) {
      setError(handleApiError(err, "Couldn't remove that application."));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <main className="mx-auto max-w-4xl px-4 py-8">
      <div className="flex items-baseline justify-between gap-4">
        <h1 className="text-2xl font-semibold">Applications</h1>
        <Link href="/internships" className="text-sm underline underline-offset-4">
          Back to postings
        </Link>
      </div>

      {error && (
        <p className="mt-4 rounded border border-red-500/40 bg-red-500/5 px-3 py-2 text-sm text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      {applications === null ? (
        <p className="mt-6 text-sm text-black/60 dark:text-white/60">Loading…</p>
      ) : applications.length === 0 ? (
        <p className="mt-6 rounded border border-black/10 dark:border-white/15 p-4 text-sm">
          Nothing tracked yet. Press “I applied” on a posting to start tracking it.
        </p>
      ) : (
        <ul className="mt-6 space-y-3">
          {applications.map((app) => (
            <li
              key={app.id}
              className="rounded-lg border border-black/10 dark:border-white/15 p-4"
            >
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <h2 className="font-medium truncate">{app.title}</h2>
                  <p className="text-sm text-black/70 dark:text-white/70">
                    {app.company_name}
                  </p>
                </div>
                <PostingState isLive={app.posting_is_live} />
              </div>

              <p className="mt-2 text-sm text-black/60 dark:text-white/60">
                Applied {formatDate(app.applied_at)}
                {app.source && ` · via ${app.source}`}
              </p>

              {app.notes && (
                <p className="mt-2 text-sm whitespace-pre-wrap">{app.notes}</p>
              )}

              <div className="mt-3 flex flex-wrap items-center gap-3">
                <label className="text-sm">
                  <span className="sr-only">Status</span>
                  <select
                    value={app.status}
                    disabled={busyId === app.id}
                    onChange={(e) =>
                      changeStatus(app.id, e.target.value as ApplicationStatus)
                    }
                    className="rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1 disabled:opacity-50"
                  >
                    {APPLICATION_STATUSES.map((status) => (
                      <option key={status} value={status}>
                        {APPLICATION_STATUS_LABELS[status]}
                      </option>
                    ))}
                  </select>
                </label>

                <a
                  href={app.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm underline underline-offset-4"
                >
                  Open posting
                </a>

                <button
                  type="button"
                  onClick={() => remove(app.id)}
                  disabled={busyId === app.id}
                  className="ml-auto text-sm text-red-700 dark:text-red-400 disabled:opacity-50"
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}

/**
 * Whether the underlying posting is still open.
 *
 * Renders nothing at all for `null`. An unknown state is not worth a badge, and any wording
 * we could pick ("Unknown", "Gone") reads as a problem with the *application*, which it is
 * not — the application is intact either way.
 */
function PostingState({ isLive }: { isLive: boolean | null }) {
  if (isLive === null) return null;
  return (
    <span
      className={`shrink-0 rounded px-2 py-0.5 text-xs ${
        isLive
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
          : "bg-black/5 dark:bg-white/10 text-black/60 dark:text-white/60"
      }`}
      title={
        isLive
          ? "This posting is still open"
          : "This posting has closed. Your application record is unaffected."
      }
    >
      {isLive ? "Open" : "Closed"}
    </span>
  );
}
