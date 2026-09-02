"use client";

import { useEffect, useState } from "react";
import { fetchMe } from "@/lib/authApi";
import {
  getRunHealth,
  startCollection,
  type RunProgress,
} from "@/lib/internshipsApi";

/** How often to re-check while a run is in flight. */
const POLL_MS = 3000;

/**
 * Shows whether a collection is running, and lets an admin start one.
 *
 * # Why this exists
 *
 * A running scrape and a broken one looked identical: empty list, empty run-health panel, no
 * message either way. Worse, with a six-hour cadence and no startup run, a fresh install
 * showed that same nothing for six hours. Two fixes went in — the backend now collects on
 * startup when the data is stale, and it persists each source as that source finishes rather
 * than writing everything at the end — and this is the part that makes both visible.
 *
 * `sources_done` climbing is only meaningful because of that second change. Batched, this
 * would sit at 0/9 for the whole run and then jump to 9/9, which is a spinner with extra
 * steps.
 */
export function CollectionStatus({
  onUpdate,
}: {
  /**
   * Called on every poll while a run is in flight, and once more when it finishes.
   *
   * On every poll, not just at the end, because the backend persists each source as that
   * source completes — a run that is 7/9 done already has most of its postings in the
   * database. Refreshing only at the end would show "Collecting…" above "Showing 0 of 0" for
   * the whole run, which is both wrong and the exact confusion this component exists to end.
   */
  onUpdate?: () => void;
}) {
  const [progress, setProgress] = useState<RunProgress | null>(null);
  const [isAdmin, setIsAdmin] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchMe()
      .then((user) => {
        if (!cancelled) setIsAdmin(user?.is_admin ?? false);
      })
      // Not being able to tell just means no button; the status still renders.
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  // Poll while a run is in flight, and once on mount to find out whether one is. The interval
  // is cleared as soon as nothing is running, so a quiet page costs one request.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let wasRunning = false;

    const check = async () => {
      try {
        const health = await getRunHealth(1);
        if (cancelled) return;
        setProgress(health.in_progress);
        // Refresh the parent whenever a run is active, and once on the tick that finds it
        // finished, so the final source's postings land in the view too.
        if (health.in_progress || wasRunning) onUpdate?.();
        wasRunning = Boolean(health.in_progress);
        if (health.in_progress) timer = setTimeout(check, POLL_MS);
      } catch {
        // A failed poll is not worth an error banner — it only drives an indicator.
      }
    };

    void check();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [onUpdate]);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      // Deliberately not awaited for the UI's sake: an uncapped run can take half an hour,
      // and the poll above is what reports progress. The promise is still handled so a
      // rejection surfaces rather than becoming an unhandled rejection.
      startCollection().catch((err: unknown) => {
        setError(err instanceof Error ? err.message : "Collection failed.");
      });
      // Give the backend a moment to write its `collection_runs` row before the next poll,
      // so the indicator appears immediately rather than one poll interval later.
      await new Promise((resolve) => setTimeout(resolve, 400));
      const health = await getRunHealth(1);
      setProgress(health.in_progress);
    } finally {
      setStarting(false);
    }
  };

  const pct = progress
    ? Math.round(
        (progress.sources_done / Math.max(1, progress.sources_total)) * 100,
      )
    : 0;

  // The banner and the button render together, never one instead of the other.
  //
  // They used to be mutually exclusive, and that turned a single stuck run into a lockout: an
  // abandoned run reported as permanently in flight, the banner replaced the button, and the
  // only control that could have started a real collection was hidden behind a collection
  // that was not happening. The backend now reconciles abandoned runs at startup, and this is
  // the second line of defence — no UI state should be able to remove the recovery path.
  return (
    <>
      {progress && (
      <section
        aria-live="polite"
        className="mt-4 rounded-lg border border-blue-500/40 bg-blue-500/5 p-3 text-sm"
      >
        <div className="flex items-center justify-between gap-3">
          <span>
            <span className="font-medium">Collecting…</span>{" "}
            <span className="text-black/60 dark:text-white/60">
              {progress.sources_done} of {progress.sources_total} sources ·{" "}
              {progress.postings_so_far} postings so far
            </span>
          </span>
          <span className="tabular-nums text-black/45 dark:text-white/45">
            {pct}%
          </span>
        </div>
        <div
          className="mt-2 h-1.5 overflow-hidden rounded bg-black/10 dark:bg-white/15"
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className="h-full bg-blue-600 transition-[width] duration-500"
            style={{ width: `${pct}%` }}
          />
        </div>
        <p className="mt-2 text-xs text-black/50 dark:text-white/50">
          Postings appear as each source finishes — you don&apos;t have to wait for the whole
          run. Sources are polled one host at a time to stay polite, so a full run takes
          around ten minutes.
        </p>
      </section>
      )}

      {isAdmin && (
    <div className="mt-4 flex items-center gap-3 text-sm">
      <button
        type="button"
        onClick={start}
        disabled={starting}
        className="rounded border border-black/15 dark:border-white/20 px-3 py-1.5 disabled:opacity-50"
      >
        {starting ? "Starting…" : progress ? "Start another run" : "Collect now"}
      </button>
      <span className="text-xs text-black/50 dark:text-white/50">
        Fetches every source. Also runs automatically on startup when the data is stale.
      </span>
      {error && (
        <span className="text-xs text-red-700 dark:text-red-400">{error}</span>
      )}
    </div>
      )}
    </>
  );
}
