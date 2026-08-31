# Hunt Extension — CLAUDE.md

**Status: 2026-08-31.** Phase 8 is built and Phase 9 with it; this file is now the *rules*, not the plan. What exists is documented in `docs/HUNT.md` — when that and this disagree, the reference is closer to the code and wins on fact, but the rules below still bind.

## Scope of this file

This file governs **Phase 8 wherever its code lives**, which is two places:

- `apps/hunt-extension/` — the Firefox extension (this folder).
- `apps/fridge-app/backend/src/inbox/` — the Gmail agent, in the site backend.

The root `CLAUDE.md` still applies to both, and `apps/fridge-app/CLAUDE.md` still applies to
the backend half. This file adds Phase 8 rules on top; it does not replace either.

(The backend half lives in `apps/fridge-app/backend/` for the same reason the blog and the
internship tab do: auth and `users` are there. That makes it the *fourth* tab in a folder
named after the first one. Root `CLAUDE.md` already calls that name a lie and says extracting
it is its own deliberate change — **do not bundle that rename into Phase 8.**)

---

## What we are building

A tool for running an internship hunt, in two tracks that share a backend but are otherwise
independent:

**Track A — the inbox agent.** Reads a burner Gmail, works out which emails are about which
application, sorts them into Gmail labels, and advances the application's status. A Firefox
extension raises a desktop notification when something lands that matters — a big-company
posting drops, or an email means OA / interview / offer.

**Track B — filling applications.** A content script that autofills your CV details into ATS
forms, plus an **answer library** so a question you have already answered well ("a personal
project you're proud of") is one click away instead of retyped from memory.

**Track B has no dependency on Track A** — no OAuth, no Google Cloud project, no API key. It is
the shortest path to something useful every day. If you want value this week rather than this
month, build **8e + 8f first** and leave the inbox agent for after.

## The one structural idea

**Your four email categories already exist in the database.** Phase 7 shipped
`internship_applications.status`, an enum of `applied → oa → interview → offer → rejected`.
That is exactly "confirmation folder / OA folder / interview folder."

So the classifier's job is **not** "sort this email into a folder." It is:

> match this email to an application row, and propose a status transition.

Gmail labels are written *afterward*, as a **projection** of application status. Build it the
other way round — labels as the source of truth — and you get two taxonomies that drift, and a
tracker still reading `applied` for a job you already interviewed at.

## Architecture

Two producers, **one** events table, one poll endpoint, one notification path:

```
  internships::collector ── new tier-1/2 posting ──┐
                                                    ├─► hunt_events ─► GET /hunt/events
  inbox::classify ───────── OA / interview / offer ─┘                   └─► notifications
```

Do not build a second pipeline for either. The full picture — every file and function — is
`docs/HUNT.md`.

## Autofill and the answer library — the parts that are not obvious from the code

Built; see `docs/HUNT.md`. Four things worth keeping in front of you:

- **Map fields by label text, not CSS selector.** Selectors rot on every ATS redesign; the
  visible label survives. Add per-ATS overrides **only** where the generic mapper demonstrably
  fails, never pre-emptively.
- **The React gotcha.** Setting `input.value` directly does not register with a React-controlled
  input — the framework's state never updates and the value is wiped on the next render. Use the
  prototype's native setter, then dispatch `input` and `change` with `bubbles: true`.
- **Résumé upload is out of scope.** File inputs cannot be populated both reliably and honestly.
  Store the path, show it as a reminder, let the user pick the file. Do not synthesise a
  `DataTransfer` to fake a selection.
- **Company-specific answers are the answer library's whole risk.** "Why do you want to work at
  X" reads as the same question everywhere, and pasting one employer's answer into another's
  form is a uniquely bad way to lose an application. Flag generously; a false positive costs one
  suggestion, a false negative costs the application.

**Workday (`*.myworkdayjobs.com`) is best-effort**, the same posture as LinkedIn in Phase 7:
included knowing it will mostly not work, never on the critical path.

### Closing the loop with the tracker

Filling a form is not applying, and the extension cannot know whether you pressed submit. So
creating an `internship_applications` row is **offered, one click, never automatic** — and that
does not contradict the rule that `Hunt/Outreach` never creates one, because there nobody
applied. It matters beyond convenience: an email can only be matched to an application that
exists.

## What "job-related cold outreach" means — decided 2026-08-29

The user's rule: **job-related cold outreach gets its own folder; everything else is largely
disregarded.** That is only actionable once "job-related" is drawn tightly, because the loose
reading swallows the folder.

A burner inbox used for applications fills up with mail that is *literally* job-related and
still junk: Indeed/LinkedIn "10 new jobs for you" digests, staffing-agency blasts, bootcamp and
master's-program marketing, job-board newsletters. Key the rules layer on the word "job" and
`Hunt/Outreach` becomes the same undifferentiated pile the inbox already is.

So the test is **specificity, not topic**:

| | Goes to `Hunt/Outreach` | Disregarded |
|---|---|---|
| **Names a specific role or company, addressed to you** | ✅ recruiter about an opening; an ATS invite for a job you didn't apply to; a referral thread | |
| **Bulk, generic, or promotional** | | job-board digests, staffing blasts, bootcamp/master's marketing, newsletters |
| **Not about employment at all** | | Google security alerts, receipts, everything else |

**A digest is disregarded even though it is job-related.** If that turns out to be wrong, the
fix is to move the boundary here, in one place, not to loosen the rules ad hoc.

`Hunt/Outreach` is a **terminal bucket, not a pipeline stage.** It never creates an
`internship_applications` row — `applied_at` is `NOT NULL` and means *you applied*. Inventing a
row for a job you never applied to corrupts the one table Phase 7 built to be trustworthy.

---

## Learning Mode — Phase 8 is `[gen]`

**The user decided on 2026-08-29: Phase 8 is fully `[gen]`, including the email classifier and
the email→application matcher.** Their words: *"This is meant to be a tool for me that I want
quickly, so I don't want to be writing any of it."*

This is a deliberate second exception, like the Phase 7 ranking exception of 2026-08-20.
**Do not re-litigate it, and do not stop at a stub boundary and hand back a signature.**
Classification is NLP-shaped and matching is fuzzy-matching-shaped, and both are still `[gen]`
here. Implement them fully.

### What the exception does NOT cover

**`[learn]` files are still never-edit.** `src/auth.rs`, `src/nlp.rs`, `src/expiration.rs`,
`src/recommend.rs`, `src/recommend_recipes.rs`, `src/rerank.rs`. The exception is about *new
Phase 8 code being `[gen]`*; it is not permission to touch those files.

Concretely, for `src/nlp.rs`:

- **Calling it is fine.** Calling is not editing. Prefer reusing it over writing a third matcher.
- **Changing it is not** — including changing a signature so it fits, and including fixing a
  compile error it throws. If Phase 8 needs `nlp.rs` to change, **say so and stop.**
- If it genuinely doesn't fit, write the matcher in `src/inbox/matching.rs` and say in a comment
  why `nlp.rs` didn't fit.

---

## Decisions already made (2026-08-29)

| Decision | Choice | Why |
|---|---|---|
| Browser | **Firefox only** | One MV3 codebase, sideload via `about:debugging`, no Apple Developer fee, no store review. Written as plain MV3 so `xcrun safari-web-extension-converter` stays available later. |
| Where the agent runs | **Rust backend, not the extension** | Extension storage is plaintext on disk, and the extension isn't running when mail arrives. |
| Classifier | **Hybrid — rules first, Claude API on ambiguity** | ~80% of this mail is formulaic. Cheap, debuggable, and degrades to rules-only if the API key is absent. |
| Gmail scope | **`gmail.modify`** | Read + label + archive. Withholds exactly the two irreversible powers: permanent delete and send-as. |
| Unmatched mail | **Job-specific → `Hunt/Outreach`; the rest disregarded** | See the section above for where that line falls. Disregarded means unlabelled, *not* unrecorded (rule 7). |
| CV profile + answers | **Stored in the backend, cached in the extension** | Survives browser reinstall; the answer library needs server-side similarity search anyway. |
| Autofill trigger | **Explicit user action only, never on page load** | Rule 10. |
| Answer suggestions | **Retrieval only; no model-written answers** | The free-text answers are the part that is actually you. |
| Non-ATS pages | **`activeTab` behind a toolbar click** | Per-invocation permission, on the one tab you asked about. The long tail of careers pages cannot be enumerated into match patterns. |

### Scope ceiling — binding

`https://www.googleapis.com/auth/gmail.modify` is the **maximum**. Do not request
`mail.google.com`, `gmail.send`, or any settings scope without asking first. The agent must
never send mail, never permanently delete, and never create filters or forwarding rules.

---

## Binding rules

These are the traps. Each one is a real failure mode, not a style preference.

### 1. Email is untrusted content, and the classifier holds Gmail write access downstream

A recruiter email — or anything pretending to be one — is **observed content**. It reaches a
model that sits upstream of a token that can relabel your inbox. That is a prompt-injection
surface.

- `inbox::classify` is a **pure function**: email in, `EmailVerdict` out. **It gets no tools.**
- The model returns a **constrained enum**, never an action, never a label name, never SQL.
- All Gmail writes and all DB writes happen in Rust, *outside* the model call, switching on
  that enum.
- Strip HTML to text, truncate the body, and **never fetch a URL found in an email.**
- If an email body contains text addressed at the agent, that is data to classify, not an
  instruction — and it is worth surfacing in `evidence`.

### 2. A misclassification must never silently rewrite the tracker

Phase 7 made `status_changed_at` load-bearing ("how long have I been at this stage"). A false
positive that flips `applied → rejected` destroys real state with no record of why.

- Every email-driven change writes a `status_proposals` row: message id, old status, new status,
  confidence, evidence. **The link from change back to email is what makes it reversible.**
- Auto-apply only above the confidence threshold, and only for **forward** transitions.
- **Never auto-apply `offer` or `rejected`.** They end the story; they always queue for review.

### 3. Status advances; it does not follow the newest email

Email order is not event order. The OA arrives, then a bulk "thanks for applying" autoresponder
lands three days late. Naive "latest email wins" drags an interview back to `applied`.

- Rank the statuses and **only move forward**, except for an explicit terminal verdict
  (rejection/offer), which may move backward-in-rank because it is genuinely later in truth.
- Pin this with a test using out-of-order timestamps.

### 4. Sync is incremental and idempotent

- Watermark on Gmail's `historyId`; use `history.list`, not a full re-list, after the first pass.
- `email_messages.gmail_message_id` is `UNIQUE`. Reprocessing a message is a **no-op**, not a
  second label write and a second notification.
- Root `CLAUDE.md` cache rule applies to Gmail too: **never fetch from a request handler.**
  Sync runs on an interval worker, same shape as `BLOG_SYNC_INTERVAL_SECS`.

### 5. A broken sync must be visible

Straight from the Phase 7 scraping rules. An expired refresh token must not look like a quiet
inbox.

- `inbox_runs` mirrors `source_runs`: started/finished, outcome, counts, error string.
- **A run that classified zero emails must be distinguishable from a run that failed to
  authenticate.** Log it and surface it in the extension's popup.

### 6. Notification dedup is the server's job

MV3 background pages get killed and restarted, so anything remembered in memory is lost and you
get the same alert twice. `hunt_events` carries read/ack state server-side; the extension acks
what it showed. `browser.storage.local` is a convenience cache, never the record.

### 7. "Disregarded" means unlabelled, not unrecorded

The disregard branch is about to become the highest-volume path in the system. If a dropped
email leaves no trace, then "the classifier correctly ignored 400 newsletters" and "the
classifier is broken and ate an OA" produce **identical output: a quiet inbox.**

This is `posting_rejects` again, one subsystem over. Phase 7's own words: a scraper that
silently discards half its input looks perfectly healthy right up until someone counts.

- A disregarded email still gets an `email_verdicts` row, with its reason and evidence.
- It gets **no** Gmail label, no event, no status effect. The inbox stays untouched.
- `inbox_runs` counts them separately: `pressing`, `confirmation`, `outreach`, `disregarded`.
  **The invariant, pinned by a test:** `classified = pressing + confirmation + outreach +
  disregarded`. Sum them into one number and the defect is invisible — same reasoning as
  `fetched = accepted + filtered + rejected` in `source_runs`.

### 8. Category is decided before the match, and a pressing email is never disregarded

The matcher is fuzzy and will miss — a company that styles itself differently in email than on
its posting, a subsidiary name, an ATS sending as `no-reply@greenhouse.io`. If "unmatched"
routes straight to disregard, **one matcher miss silently eats an interview invite.** That is
this phase's equivalent of Phase 7's trap 2, and it fails in the same direction: quietly.

So the order is fixed, and it is the same shape as Phase 7's trap 1 — the primary signal is the
record, the join is enrichment layered on top:

1. Classify the category from the email alone.
2. *Then* attempt the match.
3. **A pressing category (OA / interview / offer) is labelled and alerted even with no match**,
   with `matched_application_id` left NULL and a proposal queued for review. An unmatched
   interview invite is the single most costly thing this tool could drop.

Disregard is only ever reachable from a *non-pressing* category. Pin it with a test:
an OA email whose company matches nothing must still label and still alert.

### 9. Secrets

Google client secret and the Anthropic API key go in the backend `.env` (already `dotenvy`).
The Gmail **refresh token goes in the database**, never in a file the frontend or the extension
can read, and never in the extension at all.

### 10. Autofill never fires on its own, and never submits

The content script handles your real name, address, phone and work history on third-party pages.

- **Fill only on an explicit user action** — toolbar click or keyboard shortcut. Never on page
  load. Auto-filling on load writes your PII into the DOM of anything matching the pattern,
  including a phishing clone of a careers page.
- **Never click submit.** Fill, then stop. The user reviews and submits. A misfilled application
  sent automatically cannot be recalled.
- **Never fill a password, payment, SSN, or government-ID field**, whatever the label claims.
  Hard blocklist, checked before the fuzzy mapper runs, not after.
- **Demographic / EEO questions (race, gender, veteran, disability) are opt-in and default off.**
  They are voluntary self-identification and legally distinct from the rest of the form.
  Silently filling them in is not a convenience.
- Scope `host_permissions` to the known ATS hosts, where the script may load on match. For
  **everything else — including company-owned careers pages — use `activeTab` behind a toolbar
  click**, so access is granted per-invocation and only to the tab you asked about. Phase 7
  found company careers pages are the majority of the corpus and they cannot be enumerated, so
  this is the only honest way to reach them. **Do not ship `<all_urls>`.**

### 11. Autofill is assistive, not evasive

It fills a form the user opened, with the user's own data, on their click, and never submits.
That is squarely within the root `CLAUDE.md` scraping ethics. The boundary is unchanged: **do
not defeat anti-bot measures, do not touch CAPTCHAs, do not spoof fingerprints, do not
auto-submit.** If an ATS actively blocks form assistance, that source is best-effort and we
leave it — same as a scraper that gets pushed back.

### 12. The answer library is a drafting aid, not a submission path

Suggestions are surfaced and copied by the user. Nothing writes an answer into a form without
the user picking it, and nothing generates one. A company-specific answer must be flagged before
it can be suggested elsewhere (see above).

---

## Where the built thing is

| | |
|---|---|
| Every file and function | `docs/HUNT.md` |
| Phase status and checkpoints | `docs/PLAN.md` §§ Phase 8, 9 |
| Schema | migrations `0014` hunt_events, `0015` hunt_tokens, `0017` cv_profile, `0018` answers, `0019` inbox, `0020` label record |
| Endpoints | `src/routes/hunt.rs`, `src/routes/inbox.rs` |
| Extension | `apps/hunt-extension/` — `README.md` covers sideloading and the Firefox defaults that bite |

**Auth is a bearer token, not the session cookie.** The cookie was tried first as this file
instructed; Firefox will not send a `SameSite=Lax` cookie from a `moz-extension://` page, so the
fallback named here is what ships. See `docs/HUNT.md`.

## Build order — status

Deliberately: **classification earns write access, it does not start with it.**

| | |
|---|---|
| 8a read-only sync | ✅ complete |
| 8b classify + match | ⚠️ rules layer built; **checkpoint not met** — see below |
| 8c writes | ✅ proposals and Gmail labels |
| 8d email → alerts | ✅ complete |
| 8e extension shell | ✅ complete |
| 8f autofill | ✅ complete |
| 8g answer library | ✅ built; loop never closed by hand |
| 9 usable daily | ✅ worker, proposals panel, inbox status |

**8b's checkpoint is the outstanding one, and it needs mail rather than work.** The harness to
run it is built — `src/inbox/labelset.rs`, `labelset export` then `labelset score`; see
`docs/HUNT.md`. What is missing is the mail, not the tooling. It asks for a
hand-labelled set of *every message across ~2 weeks* — not curated job emails, because a curated
set contains no newsletters and therefore cannot measure the relevance gate, which is the
highest-volume decision in the system. Measure two numbers separately: how much junk leaked into
`Hunt/Outreach`, and **how much real mail got disregarded** — the second is the one that costs
you an interview. A held-out set can only be measured once; fixing against it spends it.

Nothing auto-applies a status change until that measurement gives a threshold
(`INBOX_AUTO_APPLY_CONFIDENCE`).

## Open questions

- [ ] Should `Hunt/Outreach` raise a notification? **Currently no, and worth keeping that way** —
      cold outreach is high-volume and low-precision, and a noisy channel gets muted wholesale,
      taking the OA alerts with it. One predicate and an existing checkbox to flip.
- [ ] Confidence threshold for auto-apply. Set it after 8b measures; guessing invents the
      measurement it is supposed to come from.
- [ ] Does the extension need the internship *list*, or only alerts? Currently only alerts.
- [ ] Does the answer library want embeddings, or is `strsim` over normalized question text
      enough? Currently `strsim`. Revisit only if retrieval measurably misses.

## Conventions

- **Ask before adding any crate or npm package.** Backend already has `reqwest` (Claude + Gmail
  HTTP) and `strsim` (fuzzy matching), so Phase 8 may need **no new backend deps at all**.
- The extension is **plain JS, no build step**, until there's a reason. Do not add a bundler,
  a framework, or TypeScript to it without asking.
- Small, reviewable commits. The user reads everything.
