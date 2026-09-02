"use client";

import {
  NOT_STATED,
  daysUntil,
  formatDate,
  formatLocation,
  formatPay,
  formatTerm,
  type RankedInternship,
} from "@/lib/internshipsApi";

/**
 * How soon a deadline has to be before it is called out. Named rather than inlined, per this
 * repo's rule that every continuous threshold gets an explicit constant.
 */
const DEADLINE_SOON_DAYS = 14;

/**
 * One posting.
 *
 * The rendering rule throughout: **an absent field says so.** Pay is missing from most
 * postings and eligibility from almost all of them, so "Not stated" is the single most common
 * thing this component draws — it has to look like information rather than an error. What it
 * must never do is render an absence as a value: no `$0`, no "Onsite" for an unknown location,
 * no "Closed" for a posting with no deadline.
 */
export function PostingCard({
  ranked,
  onTrack,
  tracking,
  tracked,
}: {
  ranked: RankedInternship;
  onTrack: (postingId: string) => void;
  tracking: boolean;
  tracked: boolean;
}) {
  const { posting, score } = ranked;
  const remaining = daysUntil(posting.deadline);
  const closingSoon =
    remaining !== null && remaining >= 0 && remaining <= DEADLINE_SOON_DAYS;

  return (
    <li className="rounded-lg border border-black/10 dark:border-white/15 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="font-medium truncate">{posting.title}</h3>
          <p className="text-sm text-black/70 dark:text-white/70">
            {posting.company_name}
          </p>
        </div>
        <span
          className="shrink-0 text-xs tabular-nums text-black/50 dark:text-white/50"
          title="Composite score, 0–1"
        >
          {score.toFixed(2)}
        </span>
      </div>

      <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-1 text-sm sm:grid-cols-4">
        <Field label="Pay" value={formatPay(posting.pay)} muted={!posting.pay} />
        <Field
          label="Location"
          value={formatLocation(posting.location)}
          muted={posting.location.is_remote === null && !posting.location.raw}
        />
        <Field
          label="Term"
          value={formatTerm(posting.term_season, posting.term_year)}
          muted={!posting.term_season && posting.term_year === null}
        />
        <Field
          label="Posted"
          value={
            posting.posted_at
              ? `${formatDate(posting.posted_at)}${posting.posted_at_is_estimated ? " (est.)" : ""}`
              : NOT_STATED
          }
          muted={!posting.posted_at}
          /* An estimated date came from when we first saw the posting, not from the source.
             Flagging it stops the entire cold-start import reading as posted today. */
          title={
            posting.posted_at_is_estimated
              ? "Estimated from when this was first seen, not stated by the source"
              : undefined
          }
        />
      </dl>

      {posting.deadline && (
        <p
          className={`mt-2 text-sm ${closingSoon ? "text-amber-700 dark:text-amber-400" : "text-black/60 dark:text-white/60"}`}
        >
          Closes {formatDate(posting.deadline)}
          {remaining !== null &&
            remaining >= 0 &&
            ` — ${remaining} day${remaining === 1 ? "" : "s"} left`}
        </p>
      )}

      {/* Shown only when pay didn't parse: it's the difference between "we couldn't read it"
          and "there wasn't one", which the bare "Not stated" above cannot express. */}
      {!posting.pay && posting.pay_raw && (
        <p className="mt-1 text-xs text-black/50 dark:text-white/50">
          Source says: {posting.pay_raw}
        </p>
      )}

      <div className="mt-3 flex items-center gap-3">
        <a
          href={posting.canonical_url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-sm underline underline-offset-4"
        >
          View posting
        </a>
        <button
          type="button"
          onClick={() => onTrack(posting.id)}
          disabled={tracking || tracked}
          className="text-sm rounded border border-black/15 dark:border-white/20 px-2 py-1 disabled:opacity-50"
        >
          {tracked ? "Tracked" : tracking ? "Saving…" : "I applied"}
        </button>
      </div>
    </li>
  );
}

function Field({
  label,
  value,
  muted,
  title,
}: {
  label: string;
  value: string;
  muted?: boolean;
  title?: string;
}) {
  return (
    <div title={title}>
      <dt className="text-xs uppercase tracking-wide text-black/45 dark:text-white/45">
        {label}
      </dt>
      <dd
        className={
          muted ? "italic text-black/45 dark:text-white/45" : "tabular-nums"
        }
      >
        {value}
      </dd>
    </div>
  );
}
