# Deploying the hunt tooling — runbook

**Phase 10h.** Everything here that is not a secret lives in `deploy/`. Nothing in this repo
holds a key, a token or a real hostname; `hunt.example.com` is a placeholder throughout.

The point of deploying is not that the site is public. It is that **the inbox agent runs when
the laptop is closed** — Phase 13's checkpoint needs a fortnight of real mail, and the clock on
that only starts here. See `docs/PLAN.md` § Phases 10–13.

---

## One origin, one host

Both processes sit behind a single origin: Caddy serves the Next frontend at `/` and the Rust
backend at `/api/*`, path-stripped.

**This is not a preference.** `fridge_session` is `SameSite=Lax`. A split origin
(`app.example.com` + `api.example.com`) makes every API call cross-site, so the cookie must
become `SameSite=None; Secure` to survive — a strictly weaker cookie, for no gain. One origin
keeps `Lax` working and makes CORS irrelevant for the site itself. `docs/PLAN.md` § After
Phase 5 reached the same conclusion from the other direction.

The Firefox extension is unaffected either way: it authenticates with a `hunt_tokens` bearer
precisely because no cookie can reach a `moz-extension://` page.

## Host layout

Adopted from `apps/fridge-app/backend/ops/` (Phase 10i) rather than invented here — the two
tasks briefly had different answers, which is precisely how a backup ends up snapshotting a
database nobody writes to.

```
/opt/personal-website/            the checkout, built in place — NOT a dated release directory
/etc/personal-website/backend.env     secrets, mode 600, owned by `hunt`
/etc/personal-website/frontend.env    optional, non-secret build/runtime knobs
/var/lib/personal-website/fridge.db   the database, outside every checkout
/var/log/caddy/hunt.log   access log; app logs go to the journal
```

### Where the tier file is found

`CompanyTiers::load()` backs the tier-1/2 alert predicate, and losing it is silent: prestige
degrades to derived-only and notifications stop, which is indistinguishable from a quiet job
market. It used to resolve **only** against `env!("CARGO_MANIFEST_DIR")` — the path the binary
was built in — which made the deployed layout load-bearing: a `releases/<date>/` directory that
is later pruned leaves the compiled-in path dangling.

Fixed 2026-09-02. It now tries, in order: `INTERNSHIP_COMPANY_TIERS`, then
`data/internships/company-tiers.json` beside the executable, then the build path. Building in
place at `/opt/personal-website` still works with no configuration; a release-directory layout
needs `INTERNSHIP_COMPANY_TIERS` set, and that is now a supported choice rather than a trap.

Check it after every deploy, because this is the one subsystem whose failure looks like success:

```bash
journalctl -u personal-website-backend --since "5 min ago" | grep -i "company tier"
```

`company tiers loaded from /…` is what you want. `NO COMPANY TIER FILE` names every path it
tried and means alerting is degraded.

---

## First deploy

Ordered. The two gates are marked and are not optional.

1. ~~**Gate — merge `phase-10-hardening`.**~~ **Done 2026-09-02.** 10g, 10i and 10j are in;
   the combined suite is 758 passing, which is exactly the two branches' additions with nothing
   lost, plus clippy at 36 and a clean `tsc`/`eslint`.
2. **Gate — backups exist and a restore has been rehearsed** (task 10i). After this deploy the
   database on this host is the only copy of a hunt in progress.
3. Create the user and directories:
   ```bash
   sudo useradd --system --home /opt/personal-website --shell /usr/sbin/nologin personal-website
   sudo install -d -o personal-website -g personal-website /opt/personal-website /var/lib/personal-website /etc/personal-website
   ```
4. Clone to `/opt/personal-website` as `hunt`.
5. `sudo install -m 600 -o personal-website -g personal-website deploy/env.production.example /etc/personal-website/backend.env`,
   then fill it in. Read the comments; each one names the failure that value causes when wrong.
6. **Run the preflight before anything else starts:**
   ```bash
   sudo -u personal-website /opt/personal-website/deploy/preflight.sh /etc/personal-website/backend.env
   ```
   It refuses on: `COOKIE_SECURE` not on, a wildcard or still-localhost `ALLOWED_ORIGINS`, a
   relative `DATABASE_URL` or one inside a checkout, half-configured Google credentials, an
   unparseable sync interval, and an unset `INBOX_APPLY_LABELS` (decided ON — see below; the
   check remains so the value is never inherited).
7. Build both halves:
   ```bash
   cargo build --release --manifest-path apps/fridge-app/backend/Cargo.toml
   ```
   ```bash
   cd frontend && NEXT_PUBLIC_FRIDGE_API_URL=https://hunt.example.com/api npm ci && npm run build
   ```
   **`NEXT_PUBLIC_*` is baked in at build time.** Setting it in the service file does nothing —
   the browser bundle already contains whatever was set when `next build` ran. Getting this
   wrong points the deployed site at `https://hunt.example.com:8080`, which the proxy does not
   serve, and every call fails as a bare network error.
8. Install the units and the proxy config from `deploy/`, then:
   ```bash
   sudo systemctl enable --now personal-website-backend personal-website-frontend caddy
   ```
9. Register both redirect URIs in Google Cloud Console — they must match **exactly**:
   `https://hunt.example.com/api/auth/google/callback` and
   `https://hunt.example.com/api/auth/gmail/callback`. A mismatch is an opaque
   `redirect_uri_mismatch`.
10. Sign in, connect Gmail, and confirm the checks under **Is it alive?** below.

## Redeploy

```bash
sudo -u personal-website git -C /opt/personal-website pull
```
Then rebuild both halves exactly as in step 7 — **the frontend must be rebuilt, not just
restarted**, whenever `NEXT_PUBLIC_FRIDGE_API_URL` or any frontend code changes — re-run the
preflight, and `sudo systemctl restart personal-website-backend personal-website-frontend`.

Migrations run at boot inside `db::init_pool`. **An applied migration is immutable**: sqlx
compares checksums and refuses to start with `migration N was previously applied but has been
modified`. If that appears after a pull, the fix is a new migration, never an edit.

## Rollback

```bash
sudo -u personal-website git -C /opt/personal-website checkout <previous-sha>
```
then rebuild and restart. **Rolling back code does not roll back a migration**, and this
project has no down-migrations. If the bad deploy added one, restore the database from the
backup taken before the deploy (gate 2) — which is why that gate exists.

## Where the database lives

`/var/lib/personal-website/fridge.db`, plus its WAL and shared-memory files. Never inside the checkout: a
relative `DATABASE_URL` resolves against `WorkingDirectory` and silently creates a second,
empty database, which reads as "all my applications vanished".

Back up the WAL alongside the database, or use `sqlite3 … ".backup"`, which is consistent by
construction. Copying `fridge.db` alone while the service is running can capture a torn state.

### One thing not to change without checking

`apiBase()` (`frontend/src/lib/apiClient.ts`) falls back to `http://127.0.0.1:8080` when there
is no `window` — server-side. Nothing calls the backend from the server today (`proxy.ts`
deliberately does not), so this never fires. If server-side code is ever added, note that
`NEXT_PUBLIC_FRIDGE_API_URL` would send it out through the public origin and back in, which
works but hairpins through Caddy and fails outright under split-horizon DNS.

## Is it alive?

Logs:

```bash
journalctl -u personal-website-backend -f
```

The inbox agent, from outside the extension — the popup's inbox line is one view of this, and
should not be the only one:

```bash
curl -s -H "Authorization: Bearer $HUNT_TOKEN" https://hunt.example.com/api/hunt/inbox/status
```

`GET /hunt/inbox/status` requires a session or a hunt token and makes no network call of its
own: it reports what the last run recorded. What you want to see is a recent run with
`outcome: success` and an advancing `historyId` watermark. **A stale timestamp is the failure
this endpoint exists to expose** — the Gmail token expires after 7 days, and the difference
between noticing in an hour and noticing in a fortnight is whether anybody looks.

---

## Required code changes — all done as of 2026-09-02

**Nothing in this section blocks the deploy any more.** It is kept struck through rather than
deleted: each row is a defect that only became visible once someone wrote down what deploying
actually involves, which is the argument for writing the runbook before the deploy rather than
during it. All three were found by 10h and fixed by the sessions after it.

| What | Where | Why it matters |
|---|---|---|
| ~~The extension cannot reach a deployed backend~~ | `apps/hunt-extension/manifest.json` | **Done 2026-09-02.** `optional_host_permissions` lets any https origin be *requested*; the extension asks for exactly the backend URL in Settings, from the **Test connection** click. No manifest edit or reload per hostname, and no code change when the host moves |
| ~~The rate limiter cannot see the caller behind a proxy~~ | `apps/fridge-app/backend/src/rate_limit.rs` | **Done 2026-09-02.** `X-Forwarded-For` is trusted only when the TCP peer is loopback, taking the last hop. From any other peer it is ignored, which is the property a direct-exposure deployment depends on |
| ~~The tier file path is compiled in~~ | `src/internships/prestige.rs` | **Done 2026-09-02.** `INTERNSHIP_COMPANY_TIERS`, then beside the executable, then the build path — and it now logs which one it read |
| ~~Three write paths still open *deferred* transactions~~ | `internships/expiry.rs`, `routes/auth.rs` ×2 | **Done, and the original claim here was too strong.** Measured: all three passed under contention as they were, because a deferred transaction whose *first* statement is a write takes the lock there and the busy handler waits. The instant-failure case (`SQLITE_BUSY_SNAPSHOT`, no wait) is specific to **read-then-write** transactions — which is what `decide` was, and why it lost 5 of 8 concurrent writes. All three now use `db::begin_write` so that a future edit adding a SELECT above the first write cannot silently reintroduce it |

---

## Pre-deploy risks

Every item here is fine on a laptop and becomes real on a public host. Each says what it costs
and what to set; none is decided silently.

### The `moz-extension://` wildcard

`routes::is_allowed_origin` admits **any** `moz-extension://` origin, not one pinned UUID.
`docs/HUNT.md` records the reasoning and calls it, in as many words, "a local-development
posture rather than a deployable one": pinning the UUID would mean a per-machine `.env` edit
that breaks on a new Firefox profile.

**The cost, on a public host:** any Firefox extension the user installs can call this API with
their session cookie. Firefox narrows it — the user must grant a host permission per extension
— but the boundary is a browser prompt, not the backend.

**Recommended:** pin the UUID. It is one origin in `ALLOWED_ORIGINS` and one `about:debugging`
lookup, and the "breaks on a new profile" cost is a one-line edit on a machine you own, paid
once, versus a standing hole on a host anyone can reach. Second best: keep the wildcard and
put the whole site behind a VPN or Tailscale, which makes the argument moot.

### `hunt_tokens` never expire

By design, and the reasoning in `docs/HUNT.md` is sound for what it was: a session expires
because it rides in a browser you walk away from, while this is a device credential, and a
clock nobody watches would stop the notifier weeks later — a failure indistinguishable from a
quiet job market. Revocation is the control.

**Reassessed for a public host:** the credential is now bearer-only, as powerful as a session,
valid forever, and revocable only through a panel nobody visits. The reasoning still holds and
the exposure is larger.

**Recommended:** keep no expiry, and mint exactly one token, labelled with the machine. Revoke
and re-mint when that machine changes. Revisit only if a second device ever needs one.

### `INBOX_APPLY_LABELS` — decided 2026-09-03: **ON**

**Task 10k is closed. The value is `true`, and `deploy/env.production.example` ships it that
way.** The owner made the call: the agent writes labels into the real mailbox from the first
deploy, rather than waiting for the classifier to be graded.

What that buys and what it costs, recorded so the decision is legible later:

- **It is reversible in the place it happens.** A wrong label is visible in Gmail and removable
  by hand. `labels.rs` never removes a label, never archives, and never touches a disregarded
  message, and the `gmail.modify` scope withholds permanent delete and send entirely.
- **Expect some wrong labels.** 8b's checkpoint is still unmet, so the classifier's accuracy is
  genuinely unmeasured until 13c grades the corpus. Labels applied before then are a guess being
  written down, and the mail this happens to is the same mail Phase 13 will be labelled from —
  which is fine, because 13b labels from **stored verdicts and raw mail, not from Gmail labels**,
  so the measurement cannot be contaminated by the agent's own writes.
- **The alternative would have been `false` for a fortnight**, which was the previous
  recommendation here. It was rejected in favour of getting the tool useful immediately.

The preflight still refuses to start without an explicit value. That check stays: it stops the
setting being *inherited* by a future host, which is a different failure from being undecided.

### The Gmail refresh token now lives on a machine you do not sit in front of

`/etc/personal-website/backend.env`, mode 600, owned by the service user, `ProtectHome` and
`ProtectSystem=strict` in the unit. It grants `gmail.modify` on the burner account.

**Recommended:** keep it the burner, never a primary mailbox. If the host is ever suspected,
revoking the app in the Google account is faster and more complete than rotating the file.

### The rate limiter behind a proxy — read this before going live

10j's limiter reads the **TCP peer** and deliberately does not trust `X-Forwarded-For`, because
a caller-controlled header would make the IP limit optional. That is the right default. Behind
Caddy, the TCP peer is always `127.0.0.1`, so:

- **every caller on the internet shares one IP bucket** of 10 login attempts per 15 minutes;
- an attacker can therefore exhaust it and **lock the real user out of logging in**, which is a
  denial of service created by the anti-denial-of-service measure;
- the per-account bucket still works normally, and is what actually protects the password.

`rate_limit.rs`'s own module docs name the proxy sharing; this is what it costs once a proxy
exists.

**Resolved 2026-09-02.** The limiter now trusts `X-Forwarded-For` only when the TCP peer is
loopback, and takes the last hop. Two properties keep that honest, and both matter:

- **From a non-loopback peer the header is ignored entirely**, so a directly-exposed
  deployment is exactly as safe as before. This is the assertion to keep if the file is ever
  rewritten — `a_forwarded_header_is_ignored_from_a_remote_peer`.
- **`deploy/Caddyfile` sets the header rather than appending to it**, so a header forged by a
  remote client does not survive the hop. If the proxy is ever swapped for one that appends,
  the last-hop rule still holds, but check that assumption rather than inheriting it.

The per-account bucket is untouched — it is what actually protects the password, and it was
never the part the proxy broke.

---

## The `[you]` half of 10h, in order

Nothing below can be done by an agent: it needs an account, a card, DNS, or a decision.

1. ~~Gate — merge `phase-10-hardening`~~ — done.
2. **Gate — drill the restore on the host** (10i). The scripts and a dev-machine drill exist;
   what is still yours is one restore performed on the deployed host, with the backend stopped.
3. ~~Decide **`INBOX_APPLY_LABELS`**~~ — decided 2026-09-03: **ON**, and the example env file
   ships `true`. Nothing to do but keep the explicit value when you fill in the real file.
4. Decide the **`moz-extension://`** posture: pin the UUID, or put the host behind a VPN.
5. Buy/choose the host and point DNS at it. Caddy provisions the certificate on first start,
   which requires the DNS record to already resolve.
6. Create the Google Cloud OAuth client and register **both** redirect URIs.
7. Fill `/etc/personal-website/backend.env` from `deploy/env.production.example` and run the preflight.
8. Deploy per **First deploy**, then confirm **Is it alive?** — including the company-tier line.
9. **Migration `0025` merges rows on the first boot after this deploy.** It re-keys 413
   postings and merges 58 duplicate groups, repointing any application that pointed at a row it
   deletes. It was verified against a copy (1,828 → 1,764 postings, zero orphans), and it is
   guarded so that a repoint it cannot make leaves the duplicate in place rather than failing —
   but it is still the first migration in this project that deletes user-visible rows. **The
   backup in gate 2 is not optional before this boot**, and `ops/backup-fridge-db.sh` is how.
10. **Run the event backfill once**, before looking at any analytics:

   ```bash
   sudo -u personal-website ./fridge_backend application-events backfill
   ```

   Without it the dashboard is not empty — it is *wrong*. `applications` is counted from the
   applications table while every conversion number comes from `application_events`, so an
   un-backfilled database shows a populated funnel in which nothing has ever responded.
   Measured on a real copy during Checkpoint 11: `responded 0 → 1`, `reached_oa 0 → 1`, with no
   indication in the response that the log had been empty.
11. Update the extension: set its Settings backend URL to `https://hunt.example.com/api` and
   press **Test connection**, which is what triggers Firefox's permission prompt for that
   origin — accept it. No manifest edit and no reload: the grant is per origin and happens at
   runtime. The backend must also name the extension's own `moz-extension://…` origin in
   `ALLOWED_ORIGINS`; **Test connection** prints the exact value when it is missing.
12. Watch it for 48 hours without touching it. That is Checkpoint 10, and it is also when the
    corpus Phase 13 needs begins to accumulate.
