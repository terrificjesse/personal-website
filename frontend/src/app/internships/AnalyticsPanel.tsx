"use client";

/**
 * The Phase 11 application-outcomes panel.
 *
 * Every chart row is a count over an application cohort, never an event-time bucket. Rejected
 * and no-response stay visibly separate: rejection overlaps `responded`, while live/dead
 * silence partition applications that have never received a response. Combining those bars
 * into a generic "closed" outcome would make the hunt look healthier than it is.
 *
 * The initial window covers the last 90 UTC dates and ends at tomorrow's midnight because the
 * backend's `to` boundary is exclusive. This frontend default should be copied into HUNT.md by
 * the documentation lane; the controls always show the exact dates being requested.
 */

import { useEffect, useState, type FormEvent } from "react";
import {
  getHuntAnalytics,
  type HuntAnalyticsBreakdown,
  type HuntAnalyticsResponse,
  type HuntAnalyticsTotals,
} from "@/lib/internshipsApi";
import { useApiError } from "@/lib/useApiError";

type WindowDraft = {
  from: string;
  to: string;
  deadAfterDays: string;
};

type RequestedWindow = WindowDraft & { requestId: number };

type Metric = {
  key: keyof Omit<HuntAnalyticsTotals, "applications">;
  label: string;
  color: string;
};

const METRICS: Metric[] = [
  { key: "responded", label: "Responded", color: "bg-sky-500" },
  { key: "reached_oa", label: "Reached OA", color: "bg-indigo-500" },
  {
    key: "reached_interview",
    label: "Reached interview",
    color: "bg-violet-500",
  },
  { key: "offers", label: "Offers", color: "bg-emerald-500" },
  { key: "rejected", label: "Rejected", color: "bg-rose-500" },
  {
    key: "no_response_live",
    label: "No response — live",
    color: "bg-amber-500",
  },
  {
    key: "no_response_dead",
    label: "No response — dead",
    color: "bg-neutral-500",
  },
];

function utcDate(date: Date): string {
  return date.toISOString().slice(0, 10);
}

function initialWindow(): WindowDraft {
  const to = new Date();
  to.setUTCHours(0, 0, 0, 0);
  to.setUTCDate(to.getUTCDate() + 1);
  const from = new Date(to);
  from.setUTCDate(from.getUTCDate() - 90);
  return { from: utcDate(from), to: utcDate(to), deadAfterDays: "45" };
}

function asUtcBoundary(value: string): string {
  // An empty or malformed value deliberately reaches the backend and becomes its documented
  // 400. The caller surfaces that error instead of converting it into an empty chart.
  return `${value}T00:00:00Z`;
}

export function AnalyticsPanel() {
  const handleApiError = useApiError();
  const [draft, setDraft] = useState<WindowDraft>(initialWindow);
  const [requested, setRequested] = useState<RequestedWindow>(() => ({
    ...initialWindow(),
    requestId: 0,
  }));
  const [data, setData] = useState<HuntAnalyticsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const deadAfterDays = requested.deadAfterDays
          ? Number(requested.deadAfterDays)
          : undefined;
        const result = await getHuntAnalytics({
          from: asUtcBoundary(requested.from),
          to: asUtcBoundary(requested.to),
          dead_after_days: deadAfterDays,
        });
        if (!cancelled) {
          setData(result);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setData(null);
          setError(handleApiError(err, "Could not load hunt analytics."));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [requested, handleApiError]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setLoading(true);
    setError(null);
    setRequested((current) => ({ ...draft, requestId: current.requestId + 1 }));
  }

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="font-semibold">Application outcomes</h2>
          <p className="mt-1 max-w-2xl text-sm text-neutral-600 dark:text-neutral-400">
            Cohorts follow when you applied. Later responses stay with that application&apos;s
            original month.
          </p>
        </div>

        <form onSubmit={submit} className="flex flex-wrap items-end gap-2 text-sm">
          <AnalyticsControl label="From (UTC)">
            <input
              type="date"
              value={draft.from}
              onChange={(event) =>
                setDraft((current) => ({ ...current, from: event.target.value }))
              }
              className="rounded border border-black/15 bg-transparent px-2 py-1 dark:border-white/20"
            />
          </AnalyticsControl>
          <AnalyticsControl label="Before (UTC)">
            <input
              type="date"
              value={draft.to}
              onChange={(event) =>
                setDraft((current) => ({ ...current, to: event.target.value }))
              }
              className="rounded border border-black/15 bg-transparent px-2 py-1 dark:border-white/20"
            />
          </AnalyticsControl>
          <AnalyticsControl label="Dead after days">
            <input
              type="number"
              min={0}
              value={draft.deadAfterDays}
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  deadAfterDays: event.target.value,
                }))
              }
              className="w-24 rounded border border-black/15 bg-transparent px-2 py-1 dark:border-white/20"
            />
          </AnalyticsControl>
          <button
            type="submit"
            disabled={loading}
            className="rounded bg-foreground px-3 py-1.5 text-background disabled:opacity-50"
          >
            {loading ? "Loading…" : "Update"}
          </button>
        </form>
      </div>

      {error && (
        <p
          role="alert"
          className="mt-4 rounded border border-red-500/40 bg-red-500/5 px-3 py-2 text-sm text-red-700 dark:text-red-400"
        >
          {error}
        </p>
      )}

      {loading && !data ? (
        <p className="mt-4 text-sm text-neutral-500">Loading analytics…</p>
      ) : data ? (
        <div className="mt-5 space-y-6">
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            <SummaryStat label="Applications" value={data.totals.applications} />
            <SummaryStat label="Responded" value={data.totals.responded} />
            <SummaryStat
              label="No response — live"
              value={data.totals.no_response_live}
            />
            <SummaryStat
              label="No response — dead"
              value={data.totals.no_response_dead}
            />
            <SummaryStat label="Reached OA" value={data.totals.reached_oa} />
            <SummaryStat
              label="Reached interview"
              value={data.totals.reached_interview}
            />
            <SummaryStat label="Offers" value={data.totals.offers} />
            <SummaryStat label="Rejected" value={data.totals.rejected} />
          </div>

          <div className="rounded bg-neutral-100 p-3 text-sm dark:bg-neutral-900">
            <div className="text-xs uppercase tracking-wide text-neutral-500">
              Time to first response
            </div>
            {data.time_to_first_response_days.median === null ? (
              <p className="mt-1">No responses in this cohort (n=0).</p>
            ) : (
              <p className="mt-1">
                Median {formatDays(data.time_to_first_response_days.median)} · p90{" "}
                {formatDays(data.time_to_first_response_days.p90)} · n=
                {data.time_to_first_response_days.n}
              </p>
            )}
          </div>

          <p className="text-xs text-neutral-500">
            Rejected is a response. No-response is shown separately and is never folded into
            rejection or a combined &ldquo;closed&rdquo; bar.
          </p>

          <div className="grid gap-4 xl:grid-cols-3">
            <BreakdownChart
              title="By source"
              rows={data.by_source}
              labelForKey={sourceLabel}
            />
            <BreakdownChart
              title="By company tier"
              rows={data.by_tier}
              labelForKey={tierLabel}
            />
            <BreakdownChart
              title="By application month"
              rows={data.by_month}
              labelForKey={monthLabel}
            />
          </div>
        </div>
      ) : null}
    </section>
  );
}

function AnalyticsControl({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-xs text-neutral-500">{label}</span>
      {children}
    </label>
  );
}

function SummaryStat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded bg-neutral-100 px-3 py-2 dark:bg-neutral-900">
      <div className="text-xs text-neutral-500">{label}</div>
      <div className="text-xl font-semibold tabular-nums">{value}</div>
    </div>
  );
}

function BreakdownChart({
  title,
  rows,
  labelForKey,
}: {
  title: string;
  rows: HuntAnalyticsBreakdown[];
  labelForKey: (key: string) => string;
}) {
  return (
    <section className="rounded border border-neutral-200 p-3 dark:border-neutral-800">
      <h3 className="text-sm font-semibold">{title}</h3>
      {rows.length === 0 ? (
        <p className="mt-3 text-xs text-neutral-500">No applications in this window.</p>
      ) : (
        <div className="mt-3 space-y-5">
          {rows.map((row) => (
            <div key={row.key}>
              <div className="flex items-baseline justify-between gap-2 text-sm">
                <span className="font-medium">{labelForKey(row.key)}</span>
                <span className="text-xs tabular-nums text-neutral-500">
                  {row.totals.applications} application
                  {row.totals.applications === 1 ? "" : "s"}
                </span>
              </div>
              <div className="mt-2 space-y-1.5">
                {METRICS.map((metric) => (
                  <MetricBar
                    key={metric.key}
                    metric={metric}
                    count={row.totals[metric.key]}
                    applications={row.totals.applications}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function MetricBar({
  metric,
  count,
  applications,
}: {
  metric: Metric;
  count: number;
  applications: number;
}) {
  const percent = applications === 0 ? 0 : (count / applications) * 100;
  return (
    <div
      className="grid grid-cols-[7.5rem_1fr_2rem] items-center gap-2 text-[11px]"
      aria-label={`${metric.label}: ${count} of ${applications}`}
    >
      <span className="truncate text-neutral-600 dark:text-neutral-400">
        {metric.label}
      </span>
      <span className="h-2 overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800">
        <span
          className={`block h-full rounded-full ${metric.color}`}
          style={{ width: `${Math.min(percent, 100)}%` }}
        />
      </span>
      <span className="text-right tabular-nums">{count}</span>
    </div>
  );
}

function formatDays(value: number | null): string {
  if (value === null) return "—";
  return `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })} days`;
}

function sourceLabel(key: string): string {
  return key === "unknown" ? "Unknown source" : key;
}

function tierLabel(key: string): string {
  return key === "unknown" ? "Unknown tier" : `Tier ${key}`;
}

function monthLabel(key: string): string {
  const match = /^(\d{4})-(\d{2})$/.exec(key);
  if (!match) return key;
  // Construct a local calendar month rather than shifting UTC midnight into the previous
  // month for users west of Greenwich.
  const month = new Date(Number(match[1]), Number(match[2]) - 1, 1);
  return month.toLocaleDateString(undefined, { month: "short", year: "numeric" });
}
