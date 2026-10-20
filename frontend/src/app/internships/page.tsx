"use client";

import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { PostingCard } from "./PostingCard";
import { CollectionStatus } from "./CollectionStatus";
import { AnswerLibrary } from "./AnswerLibrary";
import { InboxPanel } from "./InboxPanel";
import { CvProfileEditor } from "./CvProfileEditor";
import { ExtensionAccess } from "./ExtensionAccess";
import { AnalyticsPanel } from "./AnalyticsPanel";
import { ResumeVariantsPanel } from "./ResumeVariantsPanel";
import { useApiError } from "@/lib/useApiError";
import { fetchMe, type AuthenticatedUser } from "@/lib/authApi";
import {
  createApplication,
  internshipFiltersFromSearchParams,
  internshipFiltersToSearchParams,
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

/** Back to this page after signing in. The login page accepts only same-site paths in `next`. */
const SIGN_IN_HREF = "/login?next=/internships";

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
 * 3. **The URL is the filter state.** Every backend-supported filter is parsed even when this
 *    page has no control for it yet, so a shared link reproduces the exact request. Controls
 *    replace only known filter keys and preserve unrelated query parameters; back/forward
 *    navigation therefore restores the view without a second state copy drifting from it.
 *
 * # Public listing, personal everything-else (since 2026-10-19)
 *
 * The ranked list is open to anyone; the backend serves `GET /internships` and
 * `GET /internships/sources` without a session. Everything else on this page belongs to an
 * account (tracking, collection status, the inbox, the CV, the answer library), and each of
 * those 401s for a signed-out visitor, which `useApiError` turns into a redirect to `/login`.
 * So none of them may even *mount* until `/auth/me` has said someone is signed in. Rendering
 * them optimistically and letting them fail would bounce every visitor off a public page.
 */
export default function InternshipsPage() {
  return (
    <Suspense fallback={<InternshipsPageFallback />}>
      <InternshipsPageContent />
    </Suspense>
  );
}

function InternshipsPageFallback() {
  return (
    <main className="mx-auto max-w-4xl px-4 py-8">
      <h1 className="text-2xl font-semibold">Internships</h1>
      <p className="mt-6 text-sm text-black/60 dark:text-white/60">Loading…</p>
    </main>
  );
}

function InternshipsPageContent() {
  const handleApiError = useApiError();
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const filterQuery = searchParams.toString();
  const filters = useMemo(
    () =>
      internshipFiltersFromSearchParams(new URLSearchParams(filterQuery)),
    [filterQuery],
  );
  const [data, setData] = useState<ListInternshipsResponse | null>(null);
  const [sources, setSources] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [trackingId, setTrackingId] = useState<string | null>(null);
  const [trackedIds, setTrackedIds] = useState<Set<string>>(new Set());
  const [refreshVersion, setRefreshVersion] = useState(0);
  // Rows the server has handed over so far: the next page's offset. Not `data.postings.length`,
  // which is after de-duplication and would re-request the same page forever if a whole page
  // turned out to be rows already shown.
  const [nextOffset, setNextOffset] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  // Bumped whenever the first page is re-requested. A "Show more" response that comes back
  // after the filters changed belongs to the old list and must not be appended to the new one.
  const listGeneration = useRef(0);
  // `undefined` until `/auth/me` answers, then the user or `null`. Three states, not two:
  // while it is unknown, neither the personal panels nor "Sign in to track" are shown.
  const [viewer, setViewer] = useState<AuthenticatedUser | null | undefined>(undefined);
  const signedIn = viewer != null;

  // The fetch lives inside the effect rather than in a `useCallback` the effect calls, so no
  // state is set synchronously when the effect runs.
  //
  // `cancelled` is not ceremony: filters change on every keystroke in the company box, so
  // several requests are genuinely in flight at once and they can resolve out of order. The
  // guard is what stops a slow early response from overwriting a fast later one and leaving
  // the list showing results for filters the user has already moved on from.
  useEffect(() => {
    let cancelled = false;
    listGeneration.current += 1;

    (async () => {
      try {
        const result = await listInternships(filters);
        if (cancelled) return;
        setData(result);
        setNextOffset(result.postings.length);
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
  }, [filters, handleApiError, refreshVersion]);

  useEffect(() => {
    let cancelled = false;
    fetchMe()
      .then((user) => {
        if (!cancelled) setViewer(user);
      })
      // Unreachable `/auth/me` is treated as signed out: the public list still works, and
      // guessing "signed in" would mount panels that redirect to the login page.
      .catch(() => {
        if (!cancelled) setViewer(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    listInternshipSources()
      .then(setSources)
      // A missing source list is a degraded dropdown, not a broken page — the list itself
      // still loads, so this failure must not replace it with an error.
      .catch(() => setSources([]));
  }, []);

  // Identity-stable so `CollectionStatus`'s poll effect isn't torn down and restarted on
  // every render. The counter re-runs the request without changing the bookmarkable filters.
  const refresh = useCallback(() => setRefreshVersion((version) => version + 1), []);

  const update = (patch: Partial<InternshipFilters>) => {
    const next = internshipFiltersToSearchParams(
      { ...filters, ...patch },
      new URLSearchParams(filterQuery),
    );
    const query = next.toString();
    router.replace(query ? `${pathname}?${query}` : pathname, { scroll: false });
  };

  // The listing is paged (50 rows by default; `docs/INTERNSHIPS.md`, "Pages of rows"). Each
  // page is ranked from scratch, so a collection landing between two requests can move a row
  // across the boundary. Appended rows are de-duplicated by id; a row skipped that way comes
  // back on the next filter change.
  const loadMore = async () => {
    const generation = listGeneration.current;
    setLoadingMore(true);
    try {
      const page = await listInternships(filters, { offset: nextOffset });
      if (generation !== listGeneration.current) return;
      setData((current) => {
        if (!current) return page;
        const shown = new Set(current.postings.map((row) => row.posting.id));
        const fresh = page.postings.filter((row) => !shown.has(row.posting.id));
        return { ...page, offset: 0, postings: [...current.postings, ...fresh] };
      });
      setNextOffset((offset) => offset + page.postings.length);
    } catch (err) {
      if (generation !== listGeneration.current) return;
      setError(
        handleApiError(
          err,
          err instanceof Error ? err.message : "Couldn't load more postings.",
        ),
      );
    } finally {
      setLoadingMore(false);
    }
  };

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
        {signedIn ? (
          <nav className="flex gap-4 text-sm">
            <Link href="/internships/applications" className="underline underline-offset-4">
              Applications
            </Link>
            <Link href="/internships/runs" className="underline underline-offset-4">
              Run health
            </Link>
          </nav>
        ) : viewer === null ? (
          <Link
            href={SIGN_IN_HREF}
            className="text-sm text-black/60 dark:text-white/60 underline underline-offset-4"
          >
            Sign in to track applications
          </Link>
        ) : null}
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

      {signedIn && (
        <>
          <CollectionStatus onUpdate={refresh} />

          <div className="mt-4 space-y-3">
            <AnalyticsPanel />
            <InboxPanel />
            <ExtensionAccess />
            <CvProfileEditor />
            <ResumeVariantsPanel />
            <AnswerLibrary />
          </div>
        </>
      )}

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
              {data.returned.toLocaleString()} of {data.total_live.toLocaleString()} open posting
              {data.total_live === 1 ? "" : "s"} match
              {/* Said only when it differs, so nobody wonders why the counts below disagree. */}
              {data.total_rows !== data.returned &&
                `, grouped into ${data.total_rows.toLocaleString()} rows`}
            </p>

            {data.postings.length === 0 ? (
              /* The ambiguity this page exists to avoid: say which kind of empty this is. */
              <p className="mt-4 rounded border border-black/10 dark:border-white/15 p-4 text-sm">
                {data.total_live > 0
                  ? "No postings match these filters. Try widening the pay range, or turning off the “only postings that state…” switches."
                  : data.collection
                    ? "A collection is running now — postings will appear here as each source finishes."
                    : signedIn
                      ? "No postings have been collected yet. Use “Collect now” above to fetch them, or check Run health to see whether the sources are working."
                      : "No postings have been collected yet."}
              </p>
            ) : (
              <ul className="mt-4 space-y-3">
                {data.postings.map((ranked) => (
                  <PostingCard
                    key={ranked.posting.id}
                    ranked={ranked}
                    onTrack={signedIn ? track : undefined}
                    signInHref={viewer === null ? SIGN_IN_HREF : undefined}
                    tracking={trackingId === ranked.posting.id}
                    tracked={
                      // A tracked application may point at any opening in a folded row.
                      trackedIds.has(ranked.posting.id) ||
                      ranked.other_openings.some((opening) => trackedIds.has(opening.id))
                    }
                  />
                ))}
              </ul>
            )}

            {data.postings.length > 0 && (
              <div className="mt-4 flex items-center gap-4 text-sm text-black/60 dark:text-white/60">
                <span>
                  Showing {data.postings.length.toLocaleString()} of{" "}
                  {data.total_rows.toLocaleString()}
                </span>
                {nextOffset < data.total_rows && (
                  <button
                    type="button"
                    onClick={loadMore}
                    disabled={loadingMore}
                    className="rounded border border-black/15 dark:border-white/20 px-3 py-1 text-black dark:text-white disabled:opacity-50"
                  >
                    {loadingMore ? "Loading…" : "Show more"}
                  </button>
                )}
              </div>
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
