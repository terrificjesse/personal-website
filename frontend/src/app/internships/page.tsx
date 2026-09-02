"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { PostingCard } from "./PostingCard";
import { CollectionStatus } from "./CollectionStatus";
import { AnswerLibrary } from "./AnswerLibrary";
import { InboxPanel } from "./InboxPanel";
import { CvProfileEditor } from "./CvProfileEditor";
import { ExtensionAccess } from "./ExtensionAccess";
import { useApiError } from "@/lib/useApiError";
import {
  createApplication,
  listInternshipSources,
  listInternships,
  type InternshipFilters,
  type InternshipSort,
  type ListInternshipsResponse,
} from "@/lib/internshipsApi";

const SORTS: { value: InternshipSort; label: string }[] = [
  { value: "composite", label: "Best match" },
  { value: "pay", label: "Highest pay" },
  { value: "posted", label: "Most recent" },
  { value: "deadline", label: "Closing soonest" },
  { value: "prestige", label: "Top companies" },
];

const STUDY_YEARS = ["freshman", "sophomore", "junior", "senior"];

/**
 * The internship tab.
 *
 * # Two UI rules that come straight from the data
 *
 * 1. **An empty list is ambiguous and must not be left that way.** "Nothing matched your
 *    filters" and "nothing has been collected yet" look identical unless the page says which,
 *    so `total_live` is always rendered alongside the result count.
 * 2. **The "only postings that state X" toggles are prominent, not buried.** Pay is absent
 *    from most postings and eligibility from almost all of them
 *    (`docs/INTERNSHIP_SCRAPING.md` § B), so those two switches change the result set more
 *    than any other control here. Defaulting them on would hide most of the corpus; hiding
 *    them entirely would make the pay filter look broken when unpriced postings came back.
 */
export default function InternshipsPage() {
  const handleApiError = useApiError();

  const [filters, setFilters] = useState<InternshipFilters>({
    sort: "composite",
  });
  const [data, setData] = useState<ListInternshipsResponse | null>(null);
  const [sources, setSources] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [trackingId, setTrackingId] = useState<string | null>(null);
  const [trackedIds, setTrackedIds] = useState<Set<string>>(new Set());

  // The fetch lives inside the effect rather than in a `useCallback` the effect calls, so no
  // state is set synchronously when the effect runs.
  //
  // `cancelled` is not ceremony: filters change on every keystroke in the company box, so
  // several requests are genuinely in flight at once and they can resolve out of order. The
  // guard is what stops a slow early response from overwriting a fast later one and leaving
  // the list showing results for filters the user has already moved on from.
  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const result = await listInternships(filters);
        if (cancelled) return;
        setData(result);
        setError(null);
      } catch (err) {
        if (cancelled) return;
        setError(
          handleApiError(
            err,
            err instanceof Error
              ? err.message
              : "Couldn't reach the internships API.",
          ),
        );
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [filters, handleApiError]);

  useEffect(() => {
    listInternshipSources()
      .then(setSources)
      // A missing source list is a degraded dropdown, not a broken page — the list itself
      // still loads, so this failure must not replace it with an error.
      .catch(() => setSources([]));
  }, []);

  // Identity-stable so `CollectionStatus`'s poll effect isn't torn down and restarted on
  // every render. Re-setting the filters object re-runs the fetch effect without changing
  // what is being asked for.
  const refresh = useCallback(
    () => setFilters((current) => ({ ...current })),
    [],
  );

  const update = (patch: Partial<InternshipFilters>) =>
    setFilters((current) => ({ ...current, ...patch }));

  const track = async (postingId: string) => {
    setTrackingId(postingId);
    try {
      await createApplication({ posting_id: postingId });
      setTrackedIds((current) => new Set(current).add(postingId));
      setError(null);
    } catch (err) {
      setError(
        handleApiError(
          err,
          err instanceof Error ? err.message : "Couldn't save that application.",
        ),
      );
    } finally {
      setTrackingId(null);
    }
  };

  return (
    <main className="mx-auto max-w-4xl px-4 py-8">
      <div className="flex items-baseline justify-between gap-4">
        <h1 className="text-2xl font-semibold">Internships</h1>
        <nav className="flex gap-4 text-sm">
          <Link href="/internships/applications" className="underline underline-offset-4">
            Applications
          </Link>
          <Link href="/internships/runs" className="underline underline-offset-4">
            Run health
          </Link>
        </nav>
      </div>

      <section className="mt-6 rounded-lg border border-black/10 dark:border-white/15 p-4">
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <Control label="Sort by">
            <select
              value={filters.sort ?? "composite"}
              onChange={(e) =>
                update({ sort: e.target.value as InternshipSort })
              }
              className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
            >
              {SORTS.map((sort) => (
                <option key={sort.value} value={sort.value}>
                  {sort.label}
                </option>
              ))}
            </select>
          </Control>

          <Control label="I'm a">
            <select
              value={filters.class_year ?? ""}
              onChange={(e) =>
                update({ class_year: e.target.value || undefined })
              }
              className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
            >
              <option value="">Any year</option>
              {STUDY_YEARS.map((year) => (
                <option key={year} value={year}>
                  {year[0].toUpperCase() + year.slice(1)}
                </option>
              ))}
            </select>
          </Control>

          <Control label="Location">
            <select
              value={
                filters.remote === undefined ? "" : filters.remote ? "yes" : "no"
              }
              onChange={(e) =>
                update({
                  remote:
                    e.target.value === ""
                      ? undefined
                      : e.target.value === "yes",
                })
              }
              className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
            >
              <option value="">Anywhere</option>
              <option value="yes">Remote only</option>
              <option value="no">Onsite only</option>
            </select>
          </Control>

          <Control label="Pay range (USD/hour)">
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={0}
                placeholder="min"
                value={filters.pay_min ?? ""}
                onChange={(e) =>
                  update({
                    pay_min: e.target.value ? Number(e.target.value) : undefined,
                  })
                }
                className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
              />
              <span aria-hidden className="text-black/40">–</span>
              <input
                type="number"
                min={0}
                placeholder="max"
                value={filters.pay_max ?? ""}
                onChange={(e) =>
                  update({
                    pay_max: e.target.value ? Number(e.target.value) : undefined,
                  })
                }
                className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
              />
            </div>
          </Control>

          <Control label="Company">
            <input
              type="text"
              placeholder="any"
              value={filters.company ?? ""}
              onChange={(e) => update({ company: e.target.value || undefined })}
              className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
            />
          </Control>

          <Control label="Source">
            <select
              value={filters.source ?? ""}
              onChange={(e) => update({ source: e.target.value || undefined })}
              className="w-full rounded border border-black/15 dark:border-white/20 bg-transparent px-2 py-1"
              disabled={sources.length === 0}
            >
              <option value="">All sources</option>
              {sources.map((source) => (
                <option key={source} value={source}>
                  {source}
                </option>
              ))}
            </select>
          </Control>
        </div>

        {/* The two switches that matter most. See this component's doc comment. */}
        <div className="mt-4 flex flex-wrap gap-x-6 gap-y-2 border-t border-black/10 dark:border-white/10 pt-3 text-sm">
          <Toggle
            checked={filters.pay_unknown === "drop"}
            onChange={(on) => update({ pay_unknown: on ? "drop" : undefined })}
            label="Only postings that state pay"
            hint="Most postings don't publish a salary. Off by default, or the list would look empty."
          />
          <Toggle
            checked={filters.class_year_unknown === "drop"}
            onChange={(on) =>
              update({ class_year_unknown: on ? "drop" : undefined })
            }
            label="Only postings that state eligibility"
            hint="Almost none do. A posting that says nothing hasn't said you're ineligible."
          />
        </div>
      </section>

      <CollectionStatus onUpdate={refresh} />

      <div className="mt-4 space-y-3">
        <InboxPanel />
        <ExtensionAccess />
        <CvProfileEditor />
        <AnswerLibrary />
      </div>

      {error && (
        <p className="mt-4 rounded border border-red-500/40 bg-red-500/5 px-3 py-2 text-sm text-red-700 dark:text-red-400">
          {error}
        </p>
      )}

      <div className="mt-6">
        {loading && !data ? (
          <p className="text-sm text-black/60 dark:text-white/60">Loading…</p>
        ) : data ? (
          <>
            <p className="text-sm text-black/60 dark:text-white/60">
              Showing {data.returned} of {data.total_live} open posting
              {data.total_live === 1 ? "" : "s"}
            </p>

            {data.postings.length === 0 ? (
              /* The ambiguity this page exists to avoid: say which kind of empty this is. */
              <p className="mt-4 rounded border border-black/10 dark:border-white/15 p-4 text-sm">
                {data.total_live > 0
                  ? "No postings match these filters. Try widening the pay range, or turning off the “only postings that state…” switches."
                  : data.collection
                    ? "A collection is running now — postings will appear here as each source finishes."
                    : "No postings have been collected yet. Use “Collect now” above to fetch them, or check Run health to see whether the sources are working."}
              </p>
            ) : (
              <ul className="mt-4 space-y-3">
                {data.postings.map((ranked) => (
                  <PostingCard
                    key={ranked.posting.id}
                    ranked={ranked}
                    onTrack={track}
                    tracking={trackingId === ranked.posting.id}
                    tracked={trackedIds.has(ranked.posting.id)}
                  />
                ))}
              </ul>
            )}
          </>
        ) : null}
      </div>
    </main>
  );
}

function Control({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block text-sm">
      <span className="mb-1 block text-xs uppercase tracking-wide text-black/45 dark:text-white/45">
        {label}
      </span>
      {children}
    </label>
  );
}

function Toggle({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  label: string;
  hint: string;
}) {
  return (
    <label className="flex items-start gap-2" title={hint}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-1"
      />
      <span>
        {label}
        <span className="block text-xs text-black/45 dark:text-white/45">
          {hint}
        </span>
      </span>
    </label>
  );
}
