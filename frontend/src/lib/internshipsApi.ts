import { apiFetch } from "./apiClient";

/**
 * Client for the internship tab (Phase 7).
 *
 * Type names are deliberately prefixed/distinct from the other `*Api.ts` modules, per
 * `apps/fridge-app/CLAUDE.md` — only the transport in `apiClient.ts` is shared.
 *
 * # The one thing to understand before changing anything here
 *
 * **`null` means unknown, and it is never zero.** Pay is absent from well over half of all
 * postings, and class-year eligibility from almost all of them. Every optional field below is
 * a real third state that the UI has to render as "not stated" rather than as `0`, `false`,
 * or an empty string. Rendering `pay_min ?? 0` would invent a wage of nothing; rendering
 * `is_remote ?? false` would assert an office that may not exist.
 */

// ---------------------------------------------------------------------------------------
// Postings
// ---------------------------------------------------------------------------------------

export type InternshipSeason = "summer" | "fall" | "winter" | "spring";

export type InternshipPayPeriod = "hour" | "month" | "year";

/** A pay figure, or `null`. Never partially populated — the backend enforces all-or-nothing. */
export type InternshipPay = {
  min: number;
  /** `null` means the source quoted a single figure, not a range. */
  max: number | null;
  currency: string;
  period: InternshipPayPeriod;
};

export type InternshipLocation = {
  raw: string | null;
  city: string | null;
  region: string | null;
  country: string | null;
  /** `null` = unknown, `false` = onsite, `true` = remote. Three states, not two. */
  is_remote: boolean | null;
};

export type InternshipClassYears = {
  min: number | null;
  max: number | null;
  raw: string | null;
};

export type InternshipPosting = {
  id: string;
  dedup_key: string;
  company_key: string;
  company_name: string;
  title: string;
  canonical_url: string;
  term_season: InternshipSeason | null;
  term_year: number | null;
  location: InternshipLocation;
  /** `null` = the source did not state pay. Do not coerce to 0. */
  pay: InternshipPay | null;
  /** What the source said, even when it did not parse — useful to show verbatim. */
  pay_raw: string | null;
  class_years: InternshipClassYears;
  posted_at: string | null;
  /**
   * True when `posted_at` was inferred from when we first saw the posting rather than stated
   * by the source. Worth surfacing: the whole cold-start corpus is dated the day collection
   * began, so an "estimated" badge stops that reading as a genuine flood of fresh postings.
   */
  posted_at_is_estimated: boolean;
  deadline: string | null;
  first_seen_at: string;
  last_seen_at: string;
  expired_at: string | null;
  expiry_reason: string | null;
};

/** One input's contribution to the composite score, with enough detail to explain it. */
export type InternshipInputScore = {
  value: number;
  weight: number;
  basis: string;
};

export type InternshipScoreBreakdown = {
  pay: InternshipInputScore;
  recency: InternshipInputScore;
  deadline: InternshipInputScore;
  location: InternshipInputScore;
  prestige: InternshipInputScore;
};

export type RankedInternship = {
  posting: InternshipPosting;
  score: number;
  /** Exposed so the UI can explain a ranking rather than asking anyone to trust it. */
  breakdown: InternshipScoreBreakdown;
};

export type InternshipSort =
  | "composite"
  | "pay"
  | "posted"
  | "deadline"
  | "prestige";

/**
 * What to do with postings that do not state the field being filtered on.
 *
 * This is exposed in the UI rather than defaulted silently because for pay and class year it
 * is the difference between most of the corpus and almost none of it — see
 * `docs/INTERNSHIP_SCRAPING.md` § B. `"keep"` is the backend default everywhere.
 */
export type OnUnknown = "keep" | "drop";

export type InternshipFilters = {
  sort?: InternshipSort;
  term_season?: InternshipSeason;
  term_year?: number;
  term_unknown?: OnUnknown;
  remote?: boolean;
  location?: string;
  location_unknown?: OnUnknown;
  /** A graduation year (`"2029"`) or a year of study (`"sophomore"`). */
  class_year?: string;
  class_year_unknown?: OnUnknown;
  /** Inclusive bounds, in hourly USD. */
  pay_min?: number;
  pay_max?: number;
  pay_unknown?: OnUnknown;
  company?: string;
  source?: string;
};

/**
 * A collection run that has started and not yet finished.
 *
 * The reason this exists: a running scrape used to be indistinguishable from a broken one —
 * empty tab, empty health panel, no signal either way. `sources_done` climbs as each source
 * lands, which is only meaningful because the backend persists per-source rather than writing
 * everything at the end.
 */
export type RunProgress = {
  run_id: string;
  started_at: string;
  trigger: string;
  sources_done: number;
  sources_total: number;
  postings_so_far: number;
};

export type ListInternshipsResponse = {
  /**
   * Live postings before filtering. With `returned`, this is what lets an empty list say
   * "0 of 1,881 match" instead of leaving "nothing matched" and "nothing collected yet"
   * indistinguishable — which is exactly the ambiguity this tab exists to remove.
   */
  total_live: number;
  returned: number;
  sort: InternshipSort;
  postings: RankedInternship[];
  /** Set while a collection is running, so a thin list can explain itself. */
  collection: RunProgress | null;
};

function toQuery(filters: InternshipFilters): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    // `false` is a meaningful value for `remote`, so test for null/undefined rather than
    // truthiness — `if (value)` would silently drop `remote=false` and turn "onsite only"
    // into "no location filter at all".
    if (value === undefined || value === null || value === "") continue;
    params.set(key, String(value));
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}

export async function listInternships(
  filters: InternshipFilters = {},
): Promise<ListInternshipsResponse> {
  const res = await apiFetch(`/internships${toQuery(filters)}`);
  if (!res.ok) {
    // 400 means the filters themselves were rejected (an unknown sort, an inverted pay
    // window). Surfacing that verbatim beats rendering an empty list, which would look like
    // "nothing matched".
    throw new Error(
      res.status === 400
        ? "Those filters aren't valid — check the sort and pay range."
        : `Could not load internships (${res.status})`,
    );
  }
  return res.json();
}

export async function listInternshipSources(): Promise<string[]> {
  const res = await apiFetch("/internships/sources");
  if (!res.ok) throw new Error(`Could not load sources (${res.status})`);
  return res.json();
}

// ---------------------------------------------------------------------------------------
// Applied tracker
// ---------------------------------------------------------------------------------------

export type ApplicationStatus =
  | "applied"
  | "oa"
  | "interview"
  | "offer"
  | "rejected";

export const APPLICATION_STATUSES: ApplicationStatus[] = [
  "applied",
  "oa",
  "interview",
  "offer",
  "rejected",
];

export const APPLICATION_STATUS_LABELS: Record<ApplicationStatus, string> = {
  applied: "Applied",
  oa: "Online assessment",
  interview: "Interview",
  offer: "Offer",
  rejected: "Rejected",
};

/**
 * A tracked application.
 *
 * **Every field down to `notes` is a snapshot taken when you applied, and is complete on its
 * own.** The tracker renders from these alone — see the header comment on
 * `internship_applications` in migration `0012`. Do not make any of this UI depend on the
 * posting still existing.
 */
export type InternshipApplication = {
  id: string;
  /** May be set but no longer resolve — foreign keys are not enforced in the database. */
  posting_id: string | null;
  company_name: string;
  title: string;
  url: string;
  location_raw: string | null;
  pay_min: number | null;
  pay_max: number | null;
  pay_currency: string | null;
  pay_period: string | null;
  term_season: string | null;
  term_year: number | null;
  source: string | null;
  snapshot_at: string;
  status: ApplicationStatus;
  applied_at: string;
  status_changed_at: string;
  notes: string | null;
  /**
   * `true` open, `false` closed, **`null` we cannot tell** — either never linked or the
   * posting row is gone.
   *
   * `null` must never be rendered as "closed". We don't know that, and the snapshot above is
   * still perfectly good. This is the whole reason the field is three-valued.
   */
  posting_is_live: boolean | null;
};

export async function listApplications(): Promise<InternshipApplication[]> {
  const res = await apiFetch("/internships/applications");
  if (!res.ok) throw new Error(`Could not load applications (${res.status})`);
  return res.json();
}

export async function createApplication(input: {
  posting_id: string;
  status?: ApplicationStatus;
  notes?: string;
}): Promise<InternshipApplication> {
  const res = await apiFetch("/internships/applications", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  if (res.status === 409) {
    throw new Error("You've already tracked an application to this posting.");
  }
  if (!res.ok) throw new Error(`Could not save application (${res.status})`);
  return res.json();
}

export async function updateApplication(
  id: string,
  input: { status?: ApplicationStatus; notes?: string },
): Promise<InternshipApplication> {
  const res = await apiFetch(`/internships/applications/${id}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`Could not update application (${res.status})`);
  return res.json();
}

export async function deleteApplication(id: string): Promise<void> {
  const res = await apiFetch(`/internships/applications/${id}`, {
    method: "DELETE",
  });
  if (!res.ok) throw new Error(`Could not delete application (${res.status})`);
}

// ---------------------------------------------------------------------------------------
// Run health
// ---------------------------------------------------------------------------------------

export type SourceOutcome = "success" | "partial" | "failed" | "skipped";

export type SourceRunSummary = {
  run_id: string;
  source: string;
  started_at: string;
  finished_at: string | null;
  outcome: SourceOutcome;
  fetched_count: number;
  accepted_count: number;
  /** Correctly excluded (not an internship, wrong term). Expected in bulk — not a problem. */
  filtered_count: number;
  /** Should have parsed and didn't. Every one of these is a potential bug. */
  rejected_count: number;
  /**
   * Whether this run was allowed to advance disappearance counters. Shown in the UI because
   * "succeeded but expired nothing" is a real state a person needs to be able to see.
   */
  counts_for_expiry: boolean;
  error: string | null;
};

export type SourceHealth = {
  source: string;
  last_outcome: SourceOutcome;
  last_run_at: string;
  /** `null` = has never completed a full enumeration. The loudest signal on the panel. */
  last_success_at: string | null;
  last_accepted: number;
  consecutive_failures: number;
  live_postings: number;
};

export type CollectionRunSummary = {
  id: string;
  started_at: string;
  finished_at: string | null;
  trigger: string;
  /** The process died before this run finished. Reconciled at the next startup. */
  interrupted: boolean;
  sources: SourceRunSummary[];
};

export type RunHealthResponse = {
  runs: CollectionRunSummary[];
  sources: SourceHealth[];
  in_progress: RunProgress | null;
};

/** What a finished collection did. Returned by {@link startCollection}. */
export type CollectionSummary = {
  run_id: string;
  sources_run: number;
  sources_succeeded: number;
  fetched: number;
  accepted: number;
  filtered: number;
  rejected: number;
  postings_created: number;
  postings_updated: number;
  marked_closed: number;
  swept_deadline: number;
  swept_vanished: number;
};

/**
 * Kick off a collection now. Admin only.
 *
 * Runs to completion before responding, which can take a long time on an uncapped run — the
 * caller should not block its UI on this promise beyond showing that it started. Progress is
 * observable meanwhile via {@link getRunHealth}'s `in_progress`, because the backend writes
 * each source's results as that source finishes.
 */
export async function startCollection(): Promise<CollectionSummary> {
  const res = await apiFetch("/internships/collect", { method: "POST" });
  if (res.status === 403) {
    throw new Error("Only an admin can start a collection.");
  }
  if (!res.ok) throw new Error(`Could not start a collection (${res.status})`);
  return res.json();
}

export async function getRunHealth(limit = 10): Promise<RunHealthResponse> {
  const res = await apiFetch(`/internships/runs?limit=${limit}`);
  if (!res.ok) throw new Error(`Could not load run health (${res.status})`);
  return res.json();
}

// ---------------------------------------------------------------------------------------
// Formatting helpers — shared so "not stated" is worded identically everywhere
// ---------------------------------------------------------------------------------------

/** How an absent value reads. One constant so it can't drift into "N/A" in half the UI. */
export const NOT_STATED = "Not stated";

export function formatPay(pay: InternshipPay | null): string {
  if (!pay) return NOT_STATED;
  const per = { hour: "/hr", month: "/mo", year: "/yr" }[pay.period];
  const symbol = pay.currency === "USD" ? "$" : `${pay.currency} `;
  const amount = (value: number) =>
    value >= 1000 ? value.toLocaleString() : String(value);
  return pay.max !== null && pay.max !== pay.min
    ? `${symbol}${amount(pay.min)}–${symbol}${amount(pay.max)}${per}`
    : `${symbol}${amount(pay.min)}${per}`;
}

export function formatLocation(location: InternshipLocation): string {
  if (location.is_remote === true) {
    // Most sources already say "Remote" in the location string itself ("Remote (US)"), so
    // prefixing unconditionally gives "Remote — Remote (US)". Only add the prefix when the
    // raw string doesn't already carry the word.
    if (!location.raw) return "Remote";
    return /remote/i.test(location.raw)
      ? location.raw
      : `Remote — ${location.raw}`;
  }
  return location.raw ?? NOT_STATED;
}

export function formatTerm(
  season: InternshipSeason | string | null,
  year: number | null,
): string {
  if (!season && year === null) return NOT_STATED;
  const label = season ? season.charAt(0).toUpperCase() + season.slice(1) : "";
  return [label, year].filter(Boolean).join(" ") || NOT_STATED;
}

export function formatDate(value: string | null): string {
  if (!value) return NOT_STATED;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime())
    ? NOT_STATED
    : parsed.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
}

/** Days until a deadline; negative means it has passed. `null` when there is no deadline. */
export function daysUntil(value: string | null, now = new Date()): number | null {
  if (!value) return null;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return null;
  return Math.ceil((parsed.getTime() - now.getTime()) / 86_400_000);
}

// ------------------------------------------------------------------------------------------
// Extension access tokens (Phase 8e)
// ------------------------------------------------------------------------------------------

/**
 * A token the Firefox extension authenticates with.
 *
 * The extension cannot use the session cookie: it is `SameSite=Lax`, and a request from a
 * `moz-extension://` page is cross-site, so Firefox never attaches it. These are the fallback
 * `apps/hunt-extension/CLAUDE.md` names, and they exist only because that was tried first.
 */
export type ExtensionToken = {
  id: string;
  label: string;
  created_at: string;
  /** `null` until the extension has actually used it — an unused token is visibly unused. */
  last_used_at: string | null;
};

/** A freshly minted token. `secret` is returned **once** and is never recoverable. */
export type MintedExtensionToken = ExtensionToken & { secret: string };

export async function listExtensionTokens(): Promise<ExtensionToken[]> {
  const res = await apiFetch("/hunt/tokens");
  if (!res.ok) throw new Error(`Could not load extension tokens (${res.status})`);
  return res.json();
}

export async function createExtensionToken(
  label: string,
): Promise<MintedExtensionToken> {
  const res = await apiFetch("/hunt/tokens", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ label }),
  });
  if (!res.ok) throw new Error(`Could not create a token (${res.status})`);
  return res.json();
}

export async function revokeExtensionToken(id: string): Promise<void> {
  const res = await apiFetch(`/hunt/tokens/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  if (!res.ok) throw new Error(`Could not revoke the token (${res.status})`);
}

// ------------------------------------------------------------------------------------------
// CV profile (Phase 8f)
// ------------------------------------------------------------------------------------------

/**
 * The CV details the extension autofills into ATS forms.
 *
 * **Every field is nullable and that matters at the point of use.** `null` means "never filled
 * in" and the autofill skips it; an empty string would be typed into the form as a blank,
 * which looks filled to you and empty to the recruiter. The backend collapses whitespace-only
 * input to `null` on save, so this type only ever carries real values or nothing.
 */
export type CvProfile = {
  full_name: string | null;
  first_name: string | null;
  last_name: string | null;
  preferred_name: string | null;
  email: string | null;
  phone: string | null;
  location: string | null;
  school: string | null;
  degree: string | null;
  major: string | null;
  gpa: string | null;
  graduation_month: number | null;
  graduation_year: number | null;
  github_url: string | null;
  linkedin_url: string | null;
  portfolio_url: string | null;
  work_authorization: string | null;
  /** Three-state: `null` = not stated. Never defaulted — it is a legally meaningful answer. */
  needs_sponsorship: boolean | null;
  /** Shown as a reminder when a form wants a file. **Never uploaded.** */
  resume_path: string | null;
};

export const EMPTY_CV_PROFILE: CvProfile = {
  full_name: null,
  first_name: null,
  last_name: null,
  preferred_name: null,
  email: null,
  phone: null,
  location: null,
  school: null,
  degree: null,
  major: null,
  gpa: null,
  graduation_month: null,
  graduation_year: null,
  github_url: null,
  linkedin_url: null,
  portfolio_url: null,
  work_authorization: null,
  needs_sponsorship: null,
  resume_path: null,
};

export async function getCvProfile(): Promise<CvProfile> {
  const res = await apiFetch("/hunt/profile");
  if (!res.ok) throw new Error(`Could not load your CV profile (${res.status})`);
  return res.json();
}

export async function saveCvProfile(profile: CvProfile): Promise<CvProfile> {
  const res = await apiFetch("/hunt/profile", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(profile),
  });
  if (!res.ok) throw new Error(`Could not save your CV profile (${res.status})`);
  return res.json();
}

// ------------------------------------------------------------------------------------------
// The answer library (Phase 8g)
// ------------------------------------------------------------------------------------------

/**
 * A question you have already answered, and the answer you gave.
 *
 * `is_company_specific` is the important field. "Why do you want to work at X" reads as the
 * same question everywhere, which is exactly what makes reusing it verbatim so costly — so a
 * flagged answer is never offered for a different employer. The backend decides it; nothing
 * here can override it.
 */
export type ApplicationAnswer = {
  id: string;
  question_text: string;
  answer_text: string;
  is_company_specific: boolean;
  /** Who it was written for, when known. */
  company_name: string | null;
  tags: string | null;
  /** Counted when an answer is inserted into a form, not when it is merely shown. */
  use_count: number;
  last_used_at: string | null;
  created_at: string;
  updated_at: string;
};

/** A previous version, kept so a rewrite you regret is recoverable. */
export type AnswerRevision = { replaced_at: string; answer_text: string };

export async function listAnswers(): Promise<ApplicationAnswer[]> {
  const res = await apiFetch("/hunt/answers");
  if (!res.ok) throw new Error(`Could not load your answers (${res.status})`);
  const body = await res.json();
  return body.answers ?? [];
}

export async function updateAnswer(
  id: string,
  answerText: string,
): Promise<ApplicationAnswer> {
  const res = await apiFetch(`/hunt/answers/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ answer_text: answerText }),
  });
  if (!res.ok) throw new Error(`Could not save the edit (${res.status})`);
  return res.json();
}

export async function deleteAnswer(id: string): Promise<void> {
  const res = await apiFetch(`/hunt/answers/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  if (!res.ok) throw new Error(`Could not delete that answer (${res.status})`);
}

export async function answerRevisions(id: string): Promise<AnswerRevision[]> {
  const res = await apiFetch(`/hunt/answers/${encodeURIComponent(id)}/revisions`);
  if (!res.ok) throw new Error(`Could not load the history (${res.status})`);
  const body = await res.json();
  return body.revisions ?? [];
}

// ------------------------------------------------------------------------------------------
// Status proposals (Phase 8c / 9)
// ------------------------------------------------------------------------------------------

/**
 * A status change an email implied, awaiting your decision.
 *
 * **Nothing here is applied silently.** Rule 2: a misclassification must never rewrite the
 * tracker, so every email-driven change is a proposal carrying the email that caused it —
 * and that link is what makes a wrong call reversible. `from_address`, `subject`, and
 * `evidence` are separate nullable fields shown beside the change for exactly that reason:
 * an audit trail nobody can read is not one, and a subject must never be presented as a
 * sender.
 */
export type StatusProposal = {
  id: string;
  application_id: string;
  company_name: string;
  title: string;
  from_status: string;
  to_status: string;
  /** True only if the confidence threshold was configured AND the move was forward and non-terminal. */
  applied_automatically: boolean;
  from_address: string | null;
  subject: string | null;
  evidence: string | null;
  confidence: number | null;
  created_at: string;
};

export async function listProposals(): Promise<StatusProposal[]> {
  const res = await apiFetch("/hunt/proposals");
  if (!res.ok) throw new Error(`Could not load proposals (${res.status})`);
  return res.json();
}

export async function decideProposal(id: string, accept: boolean): Promise<void> {
  const res = await apiFetch(
    `/hunt/proposals/${encodeURIComponent(id)}/${accept ? "accept" : "reject"}`,
    { method: "POST" },
  );
  if (!res.ok) throw new Error(`Could not record that decision (${res.status})`);
}

/** Whether a Gmail account is connected, and what the last sync did. */
export type InboxStatus = {
  account: string | null;
  last_run: {
    started_at: string;
    finished_at: string | null;
    outcome: string;
    /** Set on a failed or partial run. Without it, "could not authenticate" and "found nothing" are the same empty row. */
    error: string | null;
    /** True when the account was reconnected after this run — the failure has been addressed. */
    superseded_by_reconnect: boolean;
    fetched: number;
    classified: number;
  } | null;
};

export async function getInboxStatus(): Promise<InboxStatus> {
  const res = await apiFetch("/hunt/inbox/status");
  if (!res.ok) throw new Error(`Could not load inbox status (${res.status})`);
  return res.json();
}
