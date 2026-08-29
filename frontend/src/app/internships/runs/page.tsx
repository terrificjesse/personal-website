"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { useApiError } from "@/lib/useApiError";
import { CollectionStatus } from "../CollectionStatus";
import {
  formatDate,
  getRunHealth,
  type RunHealthResponse,
  type SourceHealth,
  type SourceOutcome,
} from "@/lib/internshipsApi";

/** How long a source can go without a successful run before it's called stale. */
const STALE_AFTER_DAYS = 3;

/**
 * The run-health panel.
 *
 * # Why this page exists
 *
 * A source can break *quietly*. It stops being attempted, or it starts returning zero, and
 * nothing anywhere says so — the postings simply stop arriving, and a list of postings cannot
 * show you what isn't in it. A recent-runs log doesn't fix that either, because a source that
 * has stopped running is **absent** from the log, and absence is exactly what the eye slides
 * over.
 *
 * So the per-source rollup is the primary content here and the run log is secondary. Every
 * source that has *ever* run appears, with how long it's been since it last succeeded.
 */
export default function RunHealthPage() {
  const handleApiError = useApiError();
  const [data, setData] = useState<RunHealthResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  // `loadedAt` is captured once, when the data arrives, and passed down instead of letting
  // each row call `Date.now()` while rendering. Two reasons, and the lint rule is the lesser:
  // a render that reads the wall clock is not idempotent, and rows rendered in the same pass
  // would otherwise be measuring against fractionally different "now"s.
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  /// Bumped when a collection finishes, to re-fetch the panel.
  const [reloadKey, setReloadKey] = useState(0);
  /// Identity-stable, so the status component's poll effect survives re-renders.
  const refresh = useCallback(() => setReloadKey((key) => key + 1), []);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const health = await getRunHealth();
        if (cancelled) return;
        setData(health);
        setLoadedAt(Date.now());
        setError(null);
      } catch (err) {
        if (cancelled) return;
        setError(handleApiError(err, "Couldn't load run health."));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [handleApiError, reloadKey]);

  return (
    <main className="mx-auto max-w-4xl px-4 py-8">
      <div className="flex items-baseline justify-between gap-4">
        <h1 className="text-2xl font-semibold">Run health</h1>
        <Link href="/internships" className="text-sm underline underline-offset-4">
          Back to postings
        </Link>
      </div>

      <CollectionStatus onUpdate={refresh} />

      {error && (
        <p className="mt-4 rounded border border-red-500/40 bg-red-500/5 px-3 py-2 text-sm text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      {data === null ? (
        <p className="mt-6 text-sm text-black/60 dark:text-white/60">Loading…</p>
      ) : data.sources.length === 0 ? (
        <p className="mt-6 rounded border border-black/10 dark:border-white/15 p-4 text-sm">
          No collection run has happened yet.
        </p>
      ) : (
        <>
          <section className="mt-6">
            <h2 className="text-sm font-medium uppercase tracking-wide text-black/45 dark:text-white/45">
              Sources
            </h2>
            <ul className="mt-2 space-y-2">
              {data.sources.map((source) => (
                <SourceRow
                  key={source.source}
                  health={source}
                  now={loadedAt}
                />
              ))}
            </ul>
          </section>

          <section className="mt-8">
            <h2 className="text-sm font-medium uppercase tracking-wide text-black/45 dark:text-white/45">
              Recent runs
            </h2>
            <ul className="mt-2 space-y-3">
              {data.runs.map((run) => (
                <li
                  key={run.id}
                  className="rounded-lg border border-black/10 dark:border-white/15 p-3"
                >
                  <p className="text-sm">
                    {formatDate(run.started_at)}{" "}
                    <span className="text-black/45 dark:text-white/45">
                      · {run.trigger}
                      {run.finished_at === null && " · did not finish"}
                    </span>
                  </p>
                  <table className="mt-2 w-full text-sm">
                    <tbody>
                      {run.sources.map((sourceRun) => (
                        <tr key={sourceRun.source}>
                          <td className="py-0.5 pr-3">{sourceRun.source}</td>
                          <td className="py-0.5 pr-3">
                            <OutcomeBadge outcome={sourceRun.outcome} />
                          </td>
                          <td className="py-0.5 pr-3 tabular-nums text-black/60 dark:text-white/60">
                            {sourceRun.accepted_count} kept
                          </td>
                          <td className="py-0.5 pr-3 tabular-nums">
                            {/* Rejects are a defect signal; filtered rows are normal and are
                                deliberately not shown with the same emphasis. */}
                            {sourceRun.rejected_count > 0 && (
                              <span className="text-amber-700 dark:text-amber-400">
                                {sourceRun.rejected_count} unparsed
                              </span>
                            )}
                          </td>
                          <td className="py-0.5 text-black/45 dark:text-white/45">
                            {!sourceRun.counts_for_expiry && (
                              <span title="This run was not trusted to conclude that a missing posting has closed — a failed, partial, or suspiciously empty run leaves every posting exactly as it was.">
                                didn’t expire
                              </span>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {run.sources.some((s) => s.error) && (
                    <ul className="mt-2 space-y-0.5 text-xs text-black/55 dark:text-white/55">
                      {run.sources
                        .filter((s) => s.error)
                        .map((s) => (
                          <li key={s.source}>
                            {s.source}: {s.error}
                          </li>
                        ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          </section>
        </>
      )}
    </main>
  );
}

function SourceRow({
  health,
  now,
}: {
  health: SourceHealth;
  /** Reference time captured when the data loaded — see the note in `RunHealthPage`. */
  now: number | null;
}) {
  const daysSinceSuccess =
    health.last_success_at === null || now === null
      ? null
      : Math.floor(
          (now - new Date(health.last_success_at).getTime()) / 86_400_000,
        );
  const stale =
    daysSinceSuccess !== null && daysSinceSuccess > STALE_AFTER_DAYS;
  const neverSucceeded = health.last_success_at === null;

  return (
    <li
      className={`flex flex-wrap items-center gap-x-4 gap-y-1 rounded border p-3 text-sm ${
        neverSucceeded || stale
          ? "border-amber-500/40 bg-amber-500/5"
          : "border-black/10 dark:border-white/15"
      }`}
    >
      <span className="font-medium">{health.source}</span>
      <OutcomeBadge outcome={health.last_outcome} />
      <span className="text-black/60 dark:text-white/60 tabular-nums">
        {health.live_postings} live
      </span>
      <span className="ml-auto text-black/55 dark:text-white/55">
        {neverSucceeded ? (
          /* The loudest signal on this panel: it has run and has never once completed. */
          <strong className="text-amber-700 dark:text-amber-400">
            never succeeded
          </strong>
        ) : stale ? (
          <strong className="text-amber-700 dark:text-amber-400">
            last succeeded {daysSinceSuccess} days ago
          </strong>
        ) : (
          `last success ${formatDate(health.last_success_at)}`
        )}
        {health.consecutive_failures > 0 &&
          ` · ${health.consecutive_failures} failed run${health.consecutive_failures === 1 ? "" : "s"} since`}
      </span>
    </li>
  );
}

/**
 * `skipped` is styled neutrally on purpose. A source we deliberately don't fetch — LinkedIn,
 * whose `robots.txt` is `Disallow: /` — is behaving correctly, and colouring it as a failure
 * would train the eye to ignore this panel's warnings.
 */
function OutcomeBadge({ outcome }: { outcome: SourceOutcome }) {
  const styles: Record<SourceOutcome, string> = {
    success: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
    partial: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
    failed: "bg-red-500/10 text-red-700 dark:text-red-400",
    skipped: "bg-black/5 dark:bg-white/10 text-black/55 dark:text-white/55",
  };
  return (
    <span className={`rounded px-2 py-0.5 text-xs ${styles[outcome]}`}>
      {outcome}
    </span>
  );
}
