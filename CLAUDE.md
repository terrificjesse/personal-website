# Personal Website — CLAUDE.md (= AGENTS.md)

This repo is a personal project website. It is a small multi-tab Next.js app; each tab is
a self-contained mini-project. The first tab being built is the **Fridge App**.

This file governs how **any coding agent** — Claude Code, Codex, or whatever comes next —
should work in this repo. Read it before making changes. If a subproject has its own rules
file (e.g. `apps/fridge-app/CLAUDE.md`), that file's rules take precedence for work inside
that folder, but the "Learning Mode" rules below always apply repo-wide.

**`AGENTS.md` is a symlink to this file, not a copy.** It was a copy until 2026-09-02, and it
had already drifted a whole phase behind: no `apps/hunt-extension/` in its repo map, no
`docs/HUNT.md`, no Phase 8 `[gen]` exception, and no pointer to the twelve binding rules that
govern autofill — so the agent reading it was working from a repo that stopped existing in
August. Two hand-maintained copies of one rules file diverge; a symlink cannot. **If you find
yourself editing `AGENTS.md`, you are editing this file — that is the point. Never replace it
with a copy.** (`frontend/` is the one exception: `next dev` generates and re-adds
`frontend/AGENTS.md` on its own, so there `frontend/CLAUDE.md` imports it with `@AGENTS.md`
and the direction is reversed. Leave that pair alone.)

## Repo layout

```
personal-website/
  frontend/                 # Next.js app (App Router, TypeScript). The site shell + all tabs.
    app/
      fridge/                # Fridge tab lives here (added in Phase 1)
      blog/                  # Blog tab (added Phase 6) — public reading + admin-only editor
      internships/           # Internship tab (Phase 7) — scraped SWE internship postings
      ...other tabs later, not in scope yet
  apps/
    fridge-app/
      backend/               # Rust API service. Serves the fridge app, the blog, internships,
                             #   *and* the hunt alert channel (src/hunt/, Phase 8).
      CLAUDE.md               # fridge-app-specific notes (data model, endpoints, phase status)
    hunt-extension/          # Firefox MV3 extension (Phase 8) — desktop alerts, later autofill
      CLAUDE.md               # governs Phase 8 wherever its code lives, backend half included
  content/
    blog/                    # Markdown posts, synced into the DB (Phase 6)
  docs/
    PLAN.md                  # Phased implementation plan — read this for current phase status
    BLOG.md                  # Blog + admin-permissions reference (every file and function)
    INTERNSHIPS.md            # Internship tab reference (sources, schema, ranking, expiry)
    INTERNSHIP_SCRAPING.md    # Per-source data-acquisition research — read before touching a scraper
    HUNT.md                  # Phase 8 reference (hunt_events, the alert path, the extension)
    TODO.md                  # Deferred ideas
```

The blog and internship tabs' backends live inside `apps/fridge-app/backend/` because that's
where auth and the users table already are. They are conceptually **separate tabs**, not fridge
features — don't let them bleed into each other, and don't move them without a reason.

**That directory is now three tabs deep and the name is a lie.** It is the site's backend, not
the fridge app's. Extracting it (to `apps/backend/`, or a shared crate) is a real option, but
it is a rename-and-reroute across every import and doc — do it as its own deliberate change,
never bundled into a feature phase.

The fridge app is **one tab among several**, not the centerpiece of the site. Don't let its
scope creep into styling or structuring the rest of the site — keep the site shell minimal
(nav bar + tab routing) and let each tab own its own complexity.

## Tech stack

- **Backend**: Rust, `axum` for HTTP, `sqlx` for Postgres (or `sqlite` via `sqlx` for local
  dev if that's simpler to start — confirm with the user before adding a real DB dependency).
- **Frontend**: Next.js (App Router), TypeScript, Tailwind for styling.
- **API contract**: REST + JSON to start. Don't introduce GraphQL/gRPC unless asked.

## Learning Mode — READ THIS FIRST

The user is building this project specifically to **learn**, not just to ship it. They've
called out three subsystems they want to implement themselves with minimal help:

1. **Authentication** (Phase 5 — password auth + Google OAuth)
2. **Natural language processing** — fuzzy/typo-tolerant item matching (Phase 1 — "tomato"
   vs "tomatoes" vs "tomatoe" resolving to the same fridge item). **Scoped to the fridge app
   only, since 2026-09-03** — see the exception below.
3. **Recommendation algorithms** — shopping list suggestions (Phase 2), recipe matching /
   recommendation logic (Phase 3), and the like/dislike-weighted re-ranking system
   (Phase 4)
4. **Expiration date estimation** (Phase 1 — added by the user's own request, not part of
   the original three; same rules apply)

### Rules for these three subsystems

- **Do not write the core implementation** for these unless the user explicitly asks you to
  write the code (not just "help with X"). Default assumption: they want to write it.
- You CAN: explain concepts, discuss tradeoffs between approaches (e.g. Levenshtein distance
  vs. phonetic matching vs. stemming; content-based vs. collaborative filtering), point to
  papers/crates/libraries worth reading, review code they've written and give feedback,
  write *tests* that exercise their implementation (tests describe correctness, not the
  algorithm itself), and stub out the function signature / trait / interface so it plugs
  into the rest of the app.
- If asked to "just implement it" for these three areas, pause and confirm: ask whether they
  want a full implementation or want to keep driving with you as a reviewer/tutor. Don't
  assume speed is the goal here.
- Everything else in the project (CRUD scaffolding, Rust project structure, Next.js
  components, routing, database schema/migrations, API wiring, styling, tests for
  non-learning code) — generate freely and completely. Don't make the user hand-roll
  boilerplate that isn't part of what they're trying to learn.

### What is NOT a learning area

The four subsystems above are the whole list. In particular, **the blog tab (Phase 6) is not a
learning area** — content management, markdown rendering, search, sorting, and the
markdown-file ingestion pipeline are all `[gen]`. Implement them fully; do not stop at a
boundary and hand back a stub.

The one exception inside the blog's own feature set is `auth::require_admin`, which lives in
the `[learn]` file `src/auth.rs` and is already written by the user.

**The internship tab (Phase 7) is also not a learning area — including its ranking.** This is a
deliberate, explicit exception the user made on 2026-08-20, so do not "correct" it back:
`rank_postings` is a scoring-and-ordering algorithm and therefore looks exactly like the
`[learn]` subsystems from Phases 2–4, but the user chose `[gen]` for it. Scraping, storage,
normalization, dedup, filters, ranking, the applied tracker, and the expiry sweep are all
written fully.

**Fuzzy company/title matching for the internship tab is `[gen]` too, decided 2026-09-03.**
The owner lifted it explicitly: NLP is a learning area because of the *fridge app*, and for this
tab the goal is results. So an agent may design and write matching logic under
`src/internships/**` without asking, including a second matcher — which is what
`src/internships/company_match.rs` is.

Two things this exception does **not** cover:

- **`src/nlp.rs` itself is still off-limits without asking**, and now for a different reason:
  blast radius, not pedagogy. It is live fridge behaviour with its own tests and its own users,
  and `docs/INTERNSHIP_SCRAPING.md` § C measured that its bands are tuned for one- and two-word
  grocery names and break on job titles. Retuning it to serve the internship corpus would
  degrade the fridge. If a change there is genuinely the cleanest option, make the case first.
- **The other three learning areas are untouched.** Auth, recommendations and expiration
  estimation are exactly as reserved as they were.

### Never edit a `[learn]` file

`src/auth.rs`, `src/nlp.rs`, `src/expiration.rs`, `src/recommend.rs`,
`src/recommend_recipes.rs`, `src/rerank.rs`. Diagnosis only — report what's wrong and where,
and let the user make the change. **Fixing a compile error still counts as editing.** If a
`[gen]` change requires one of these files to change, say so and stop rather than doing it.

### Phase discipline

Work phase-by-phase per `docs/PLAN.md`. Don't scaffold Phase 3 code while Phase 1 is still
in progress unless the user asks for it explicitly — this is a learning project and jumping
ahead undercuts the incremental structure that makes each phase a legible unit of learning.

## Scraping rules (Phase 7 — the internship tab)

These govern anything that fetches from a third party. They came from the user directly and
are not defaults to be re-weighed per source.

- **Per-source failure isolation is the architecture, not error handling.** One source being
  blocked, rate-limited, reshaped, or returning garbage must never fail a run or reduce what
  the other sources produced. A source returns its postings *or* it returns a recorded failure;
  the runner treats those as equally normal outcomes.
- **Fail fast, then move on.** No retry storms, no long backoff loops waiting for a source that
  has decided to block you. Give up on that source for that run.
- **Every failure lands somewhere a human will find it** — a `source_runs` row and a log line,
  with the source, the reason, and the count. A source silently returning zero postings must be
  distinguishable from a source that genuinely had zero.
- **No detection evasion.** Identify honestly in the user agent, respect `robots.txt`,
  rate-limit politely. **Do not** add proxy rotation, CAPTCHA solving, browser-fingerprint
  spoofing, headless-browser cloaking, or scraping from behind a login. The user's stated
  preference is to give up quickly when a source pushes back, which is also the only version of
  this that keeps working. If a source is only reachable by evading its controls, report that
  and leave it returning zero — don't build the workaround.
- **LinkedIn / Indeed / Handshake are best-effort and expected to yield little.** The user
  chose to include them knowing this. They must never be on the critical path for a run to be
  useful, and a run where all three fail is a normal run.
- **Cache aggressively and re-fetch rarely.** The vendored-snapshot precedent is
  `data/themealdb/` and `data/foodkeeper/`: fetch once, commit the snapshot, document when it
  was taken. Do not re-fetch on every request, and never from a request handler.
- Read `docs/INTERNSHIP_SCRAPING.md` before changing any source adapter — it records what each
  source actually provides and which fields are absent.

## Working conventions

- Prefer small, reviewable commits/diffs over large ones — the user is reading everything.
- When generating backend scaffolding, favor explicit, readable Rust over clever
  abstractions; this codebase is also a Rust-learning vehicle even outside the three
  flagged subsystems.
- Ask before adding new dependencies (crates or npm packages) beyond what's already
  agreed in `docs/PLAN.md`.
- When in doubt about whether something falls under "Learning Mode," ask rather than assume.

## Working with two coding agents (added 2026-09-02, amended 2026-09-03)

From Phase 10 the repo is worked by **two agents — Claude Code and Codex — against two
separate weekly credit budgets**. That is a throughput decision, and it introduces failure
modes a single agent does not have. These rules are binding on both.

> **Amended 2026-09-03: Claude Code took over Codex's queue, and the repo is on one agent.**
> Rules written for two agents do not all survive that, and one of them — rule 7 — became
> literally unsatisfiable. Each rule below now says what it means with one agent and what it
> assumed with two, because **a rule nobody can satisfy is worse than no rule**: it lets
> "reviewed" be written into `docs/` when nothing independent happened, which is the exact
> class of untrue claim the rest of this file exists to prevent.
>
> Nothing here is deleted. If a second agent resumes, the two-agent reading is still the one
> to use, and it is still correct.

**1. One rules file, symlinked.** See the top of this file. A rule that lives in only one
agent's copy is a rule the other agent will violate while being helpful.

**2. Lanes, and a worktree each.** Never run both agents on the same working tree.

*With one agent the worktree rule is moot and the lane table is not.* The lanes stopped being
about who works where and became **what a commit is allowed to touch at once** — which is the
half that was doing the useful work anyway. A diff spanning A and B is a diff crossing the
seam, and rule 4 still applies to it.

| Lane | Owns | Typical work |
|---|---|---|
| **A — backend** | `apps/fridge-app/backend/src/**`, `migrations/` | Rust, SQL, sources, the inbox agent |
| **B — client** | `frontend/src/**`, `apps/hunt-extension/**` | React, the extension, anything in a browser |
| **C — docs** | `docs/**`, the rules files | Reconciliation, phase write-ups, specs |

Lane C is written by whoever finishes first; it is never a *concurrent* lane, because two
agents appending to `docs/PLAN.md` at once conflict on every line of it.

**3. Migration numbers are reserved, not discovered.** An agent that picks the next free number
by looking at the directory will pick the same one as the agent in the other worktree, and the
conflict surfaces at `sqlx migrate run`, not at merge.

| Block | Reserved for | Status |
|---|---|---|
| `0021–0029` | Lane A | **exhausted** — `0029` taken 2026-09-03 |
| `0030–0059` | **Claude Code** | in use — `0030`, `0031` taken |
| `0060–0089` | **Codex** | held open, unused |
| `0090+` | unreserved | claim a block here before using one |

Codex's block stays reserved rather than being reclaimed. It costs nothing — numbers are free
— and collapsing it would mean renumbering if a second agent ever resumes, which is the one
operation this table exists to avoid.

**Reserved per agent, not per lane, since 2026-09-03.** The original scheme gave the block to
Lane A, which stopped working the moment both agents started doing backend work: two agents
inside one reserved block collide exactly as if there were no rule. The unit of reservation has
to be whoever is holding the pen.

**Exhaustion has a protocol, because the first block ran out with none.** When your block is
down to its **last two numbers**, reserve the next one *in this file* before you spend them.
Discovering there is no free number in the middle of writing a migration is what happened on
2026-09-03, and it blocked the work rather than the other way round.

Whatever the table says, the rule underneath it does not change: **never pick a number because
it looks free.**

**4. Cross-seam work is contract-first.** The backend↔client seam is in two languages and
invisible to both compilers — it is what `f17d983` spent a whole commit closing by hand. A
feature crossing it gets its request and response shapes written into `docs/HUNT.md` *before*
either lane starts, and the crossing itself is one lane's job, not two halves that meet.

**5. Every decision lands in `docs/` in the same commit that acts on it.** A decision that
exists only in one agent's transcript is invisible to the other agent permanently, and to the
user by next week. This is not documentation hygiene; it is the only shared memory the two
agents have.

**6. No task may be bottlenecked on one specific agent.** Every `[gen]` task in `docs/PLAN.md`
names a **primary** agent and is marked **swappable** unless something genuinely makes it otherwise.
Swappable means: the spec in `docs/` is complete enough that the *other* agent can pick the
task up cold, having read nothing but the named files. When one budget runs out mid-week, the
other takes the queue — so write the spec before starting, not after. The genuinely
non-swappable work is named in each phase and is almost always `[you]`: labelling mail,
loading the extension in Firefox, holding the deploy secrets, and the `[learn]` boundary calls.

*With one agent this rule paid off rather than expired.* Codex's queue transferred on
2026-09-03 with one asterisk (rule 7, below) and no rewriting, because the specs were in
`docs/` as this rule required. **Keep writing them that way.** The other agent a spec is
written for is now future-you with none of today's context, which is a harder reader, not an
easier one.

**7. The agent that wrote a diff does not review it.** Hand it to the other one. This repo's
history is a catalogue of defects that hundreds of green tests did not catch; two models miss
different things, and a review pass costs a fraction of an implementation pass.

*With one agent there is no other one, so this rule cannot be followed as written.* Deleting it
would be the wrong repair — the reason for it does not go away just because the mechanism does.
What replaces it:

- **A self-review is labelled a self-review**, in the commit and in `docs/`. Never "reviewed".
  A reader in a month has to be able to tell it apart from what Codex did to `12f`, and cannot
  if both say the same word.
- **Review against artifacts, not against reasoning.** Re-reading your own argument re-derives
  your own conclusions; it is the weakest thing you can do and it feels like the strongest.
  What still works when author and reviewer are the same person: regenerate what was generated
  and diff it, re-run every number a commit message asserts, check invariants against live data
  rather than against their own tests, and grep for claims naming a test that does not exist.
  **This is not theoretical.** On 2026-09-03 a claim that Greenhouse "can almost never expire
  anything" had survived five documents and two module docs written by the same agent that was
  re-reading them. One query — 8 successes in 16 runs — killed it.
- **The user is the remaining independent reader.** Rule 8 is what protects that, and it stops
  being a budget note and starts being the whole mechanism.

**8. Review capacity is the real budget.** Two agents double the code produced and do not
double the hours the user has to read it. The "small, reviewable commits" convention above
gets *stricter* here, not looser.

*With one agent this is more true, not less.* Output halves; the user's reading time does not
double to compensate, and rule 7's independent check is gone. The user is now the only reader
who did not write the code. **Spend that on the diffs that can lose data** — migrations, the
expiry sweep, anything that deletes or merges rows — and be willing to answer "what should I
do next" with *nothing, stop* when the honest answer is that the backlog of unread work is the
biggest risk in the project.
