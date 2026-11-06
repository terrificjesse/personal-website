# Deploying the hunt tooling — runbook

**Phase 10h.** Everything here that is not a secret lives in `deploy/`. Nothing in this repo
holds a key, a token or a real hostname; `hunt.example.com` is a placeholder throughout.

The first reason to deploy is that **the inbox agent runs when the laptop is closed** — Phase
13's checkpoint needs a fortnight of real mail, and the clock on that only starts here. See
`docs/PLAN.md` § Phases 10–13. Since 2026-10-19 there is a second: the internship listing is
public, so a deployed site is also what visitors use.

**Corrected against the code on 2026-11-05.** Nothing here has yet been run on a real host, and
§ Rehearsed says exactly which parts have and have not been run anywhere.

---

## Before you start — what this actually is

**One machine, three processes, one domain name.** Caddy listens on 443 and hands `/` to the
Next.js frontend and `/api/*` to the Rust backend. SQLite is a file on that machine's disk, not
a service. Nothing is containerised and nothing is clustered, because one person's job hunt does
not need it.

The thing you are buying is **uptime for the inbox agent**. Everything else already works on
your laptop; what does not work on a laptop is a fortnight of continuous polling.

### The shopping list

| What | Why | Notes |
|---|---|---|
| A small Linux VPS | Somewhere that stays on | **≥ 2 GB RAM and ≥ 20 GB disk.** Not a 1 GB box: `cargo build --release` links the backend in one process and will be OOM-killed, and the Rust `target/` directory alone runs to several GB |
| A domain name | Caddy provisions a TLS certificate against it, and Google's OAuth client demands an `https://` redirect | Any registrar. This is the only item with a lead time — DNS has to resolve *before* the first deploy |
| A Google Cloud OAuth client | Sign-in, and the Gmail scope the inbox agent runs on | Free. The client you already use locally is fine: add the two deployed redirect URIs to it |

Nothing else. No managed database, no object store, no CI runner.

### The order things have to happen in, and why

Three of the steps are ordered by something other than preference, and getting them out of
order costs a retry rather than data:

1. **DNS resolves → then deploy.** Caddy gets its certificate on first start by answering a
   challenge on the domain. Start it before the record propagates and it fails, backs off, and
   you wait.
2. **The frontend's API URL is baked in at build time.** `NEXT_PUBLIC_*` is compiled into the
   browser bundle. Setting it in the service file after the fact does nothing — you have to
   rebuild.
3. **The Google redirect URIs are registered before you sign in.** Both of them, matching
   character for character, or the round trip dies as an opaque `redirect_uri_mismatch`.

### The two things that are not retryable

Everything above is a retry. These two are not, and they are why the checklist has gates:

- **A restore you have never rehearsed.** After the first deploy, the copy of the database on
  that host is the only record of a hunt in progress. `ops/restore-fridge-db.sh` exists and has
  been drilled on a dev machine; drilling it *on the host*, with the backend stopped, is the
  gate.
- **A boot can run migrations that delete rows.** `0025` merges 58 duplicate groups, `0027`
  deletes 4 orphaned postings, `0029` merges another. All three were verified against copies of
  this database and all three are guarded — but "verified against a copy" and "run on the only
  copy" are different sentences. **The laptop's database already has them** (it is at `0036`), so
  bringing it over as step 9 describes runs none of them. Any database behind them does, so take
  the backup first.

### Roughly how long

The runbook is 8–12 hours, but it is not 8–12 hours of typing. Most of it is waiting: DNS
propagation, a first `cargo build --release` on a small box (20–40 minutes), and then a
deliberate 48-hour period of not touching it, which is Checkpoint 10 and also when Phase 13's
corpus starts accumulating.

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
/opt/personal-website-build/      HOME for builds: the Rust toolchain and the npm cache
/etc/personal-website/backend.env     secrets, mode 600, owned by `personal-website`
/etc/personal-website/backup.env      backup paths, mode 600
/etc/personal-website/frontend.env    optional, non-secret build/runtime knobs
/var/lib/personal-website/fridge.db   the database, outside every checkout
/var/backups/personal-website/        verified snapshots, mode 700: they hold the Gmail token
/var/log/caddy/hunt.log   access log; app logs go to the journal
```

**Why builds get their own `HOME`.** The service user's home is the checkout, whose `.cargo/` is
the repo's own config directory. Built with the default `HOME`, Cargo would put its registry and
binaries inside that directory, and npm its cache in the checkout.

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

Ordered. The gates are marked and are not optional. Commands assume Ubuntu 24.04, and
`hunt.example.com` stands for your domain throughout.

1. ~~**Gate — merge `phase-10-hardening`.**~~ **Done 2026-09-02.** 10g, 10i and 10j are in;
   the combined suite is 758 passing, which is exactly the two branches' additions with nothing
   lost, plus clippy at 36 and a clean `tsc`/`eslint`.
2. **On the laptop: merge what should go live to `main`, and push it.** `main` is what gets
   cloned. See "Which branch gets deployed" below.
3. **Register both redirect URIs in Google Cloud Console** — they must match **exactly**:
   `https://hunt.example.com/api/auth/google/callback` and
   `https://hunt.example.com/api/auth/gmail/callback`. A mismatch is an opaque
   `redirect_uri_mismatch`. See "Google's limits on Gmail" below for what else to set there.
4. **Firewall and tools.** Both processes now listen on loopback only, so the firewall is a second
   layer rather than the only one:
   ```bash
   sudo ufw allow OpenSSH && sudo ufw allow 80 && sudo ufw allow 443 && sudo ufw --force enable
   sudo apt update && sudo apt install -y build-essential pkg-config git sqlite3
   curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash - && sudo apt install -y nodejs
   ```
   Then install Caddy from [its Ubuntu instructions](https://caddyserver.com/docs/install#debian-ubuntu-raspbian).
   **Node from NodeSource, not nvm and not Ubuntu's package:** the unit hardcodes `/usr/bin/npm`,
   and the version Ubuntu packages is too old for this Next.js. The C toolchain is for SQLite,
   which is compiled into the backend. `sqlite3` is what the backup scripts run.
5. Create the user and directories:
   ```bash
   sudo useradd --system --home /opt/personal-website --shell /usr/sbin/nologin personal-website
   sudo install -d -o personal-website -g personal-website /opt/personal-website /opt/personal-website-build /var/lib/personal-website /etc/personal-website
   sudo install -d -m 700 -o personal-website -g personal-website /var/backups/personal-website
   ```
6. Clone to `/opt/personal-website` — **and name what you are cloning.** See
   "Which branch gets deployed" directly below; a bare `git clone` takes the default branch,
   which on 2026-09-03 was 87 commits behind and contained none of this. The repo is public, so
   HTTPS needs no key:
   ```bash
   sudo -u personal-website git clone https://github.com/terrificjesse/personal-website.git /opt/personal-website
   test -f /opt/personal-website/deploy/preflight.sh || echo "WRONG CHECKOUT — see step 6"
   ```
   One line, and it turns a confusing failure at the next step into an obvious one at this one.
7. Create the secrets file and fill it in. Read the comments; each one names the failure that
   value causes when wrong. Then **run the preflight before anything else starts:**
   ```bash
   sudo install -m 600 -o personal-website -g personal-website /opt/personal-website/deploy/env.production.example /etc/personal-website/backend.env
   sudo nano /etc/personal-website/backend.env
   sudo -u personal-website /opt/personal-website/deploy/preflight.sh /etc/personal-website/backend.env
   ```
   It refuses on: `COOKIE_SECURE` not on, a wildcard or still-localhost `ALLOWED_ORIGINS`, a
   relative `DATABASE_URL` or one inside a checkout, a `BIND_ADDRESS` other than `127.0.0.1`,
   half-configured Google credentials, an unparseable sync interval, and an unset
   `INBOX_APPLY_LABELS` (decided ON — see below; the check remains so the value is never
   inherited).
8. Build both halves. The first build takes 20–40 minutes on a small box. Install the Rust
   toolchain, then:
   ```bash
   sudo -u personal-website env HOME=/opt/personal-website-build sh -c 'curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path'
   sudo -u personal-website env HOME=/opt/personal-website-build PATH=/opt/personal-website-build/.cargo/bin:$PATH sh -c 'cd /opt/personal-website && cargo build --release --manifest-path apps/fridge-app/backend/Cargo.toml'
   ```
   **Run from the repo root, as written.** `.cargo/config.toml` at the root builds SQLite
   without its process-wide memory-statistics lock, and Cargo only finds config by walking up
   from the working directory. Built from anywhere that does not see it, the binary works but
   the public listing serves about a thirteenth of the traffic
   (`docs/PUBLIC_API_STRESS_TEST.md`, S1). The backend prints a `WARNING` at startup when that
   has happened — check `journalctl -u personal-website-backend` after the first start.
   ```bash
   sudo -u personal-website env HOME=/opt/personal-website-build NEXT_PUBLIC_FRIDGE_API_URL=https://hunt.example.com/api sh -c 'cd /opt/personal-website/frontend && npm ci && npm run build'
   ```
   **`NEXT_PUBLIC_*` is baked in at build time.** Setting it in the service file does nothing —
   the browser bundle already contains whatever was set when `next build` ran. Getting this
   wrong points the deployed site at `https://hunt.example.com:8080`, which the proxy does not
   serve, and every call fails as a bare network error.
9. **Bring your data over**, or skip this to start empty. Stop the laptop's backend first: two
   copies syncing one mailbox would both label it and both alert. On the laptop:
   ```bash
   sqlite3 apps/fridge-app/backend/fridge.db ".backup '/tmp/fridge-deploy.db'"
   scp /tmp/fridge-deploy.db YOU@YOUR-SERVER:/tmp/
   ```
   Then on the host. **The command-line verbs need `DATABASE_URL` spelled out:** they do not read
   `backend.env`, and without it they open a relative `fridge.db`, quietly creating an empty one.
   ```bash
   sudo install -m 600 -o personal-website -g personal-website /tmp/fridge-deploy.db /var/lib/personal-website/fridge.db
   sudo -u personal-website env DATABASE_URL='sqlite:///var/lib/personal-website/fridge.db?mode=rwc' /opt/personal-website/apps/fridge-app/backend/target/release/fridge_backend application-events verify
   ```
   **Delete both `/tmp` copies afterwards:** they hold the Gmail token and the password hashes.
   Starting empty instead, the first account to register claims any unowned rows, and admin is
   granted by hand (`apps/fridge-app/CLAUDE.md` § Admin flag).
10. **Gate — backups exist and a restore has been rehearsed** (task 10i), before the first start.
    After this deploy the database on this host is the only copy of a hunt in progress.
    ```bash
    cd /opt/personal-website/apps/fridge-app/backend/ops
    sudo install -m 600 -o personal-website -g personal-website backup.env.example /etc/personal-website/backup.env
    sudo cp systemd/fridge-db-backup.service systemd/fridge-db-backup.timer /etc/systemd/system/
    sudo systemctl daemon-reload && sudo systemctl enable --now fridge-db-backup.timer
    sudo systemctl start fridge-db-backup.service
    sudo ls -t /var/backups/personal-website
    sudo -u personal-website ./restore-fridge-db.sh /var/backups/personal-website/NEWEST-FILE /tmp/fridge-restored.db
    ```
    Read `journalctl -u fridge-db-backup.service` rather than waiting until 03:30 to learn a path
    is wrong.
11. Install the units and the proxy config from `deploy/`, then start them:
    ```bash
    sudo cp /opt/personal-website/deploy/personal-website-backend.service /opt/personal-website/deploy/personal-website-frontend.service /etc/systemd/system/
    sudo cp /opt/personal-website/deploy/Caddyfile /etc/caddy/Caddyfile
    sudo sed -i 's/hunt\.example\.com/YOUR-DOMAIN/' /etc/caddy/Caddyfile
    systemd-analyze verify /etc/systemd/system/personal-website-backend.service /etc/systemd/system/personal-website-frontend.service
    sudo systemctl daemon-reload
    sudo systemctl enable --now personal-website-backend personal-website-frontend
    sudo systemctl restart caddy
    ```
    `systemd-analyze verify` should print nothing about these two units. It is the check that
    catches a setting in the wrong section, which systemd otherwise only warns about in the journal.
12. Sign in, connect Gmail, and confirm the checks under **Is it alive?** below. Reconnect even
    with a database brought over from the laptop: its stored token keeps working until it expires,
    but reconnecting is what proves the deployed redirect URI works.

## Which branch gets deployed

**Decide this before the first clone.** It is one decision and it has bitten once already.

On 2026-09-03 `main` sat at `7dcc0fd`, the commit *before* Phase 10 began — 87 commits behind,
and missing `docs/DEPLOY.md`, every file in `deploy/`, and 11 of the 31 migrations. The runbook
said "clone" and nothing else, so the branch it told you to clone did not contain the runbook or
a single artifact it installs. The clone would have succeeded and the next step would have failed
on a missing file, three commands into a first deploy, with nothing pointing at the cause.

**Decided 2026-09-03: `main`.** Phases 10 through 12 were merged to it, so a plain `git clone`
is correct, `Redeploy`'s `git pull` is right as written, and there is nothing to remember. The
alternative — deploying a named branch, with `git clone --branch <name>` at step 6 and a matching
`pull` in `Redeploy` — was rejected as one more thing to keep in sync for no gain.

**What that decision commits you to:** `main` is now the deployed branch, so work merges there
before it is deployed, and `main` staying current is the thing that stops this trap reappearing.
It went 87 commits stale over three weeks precisely because nothing depended on it.

**Whichever you choose, the trap is the same one:** a clone takes the *default* branch, and this
repo's default branch has been stale for the entire period the deploy has been planned. Nothing
warns you. The `test -f` in step 6 is the whole defence and it costs a line.

## Redeploy

```bash
sudo -u personal-website git -C /opt/personal-website pull
```
**This pulls whatever branch the clone tracked.** If you deployed a named branch rather than
`main`, say so here in your own copy — a `pull` that silently tracks the wrong branch is the
same failure as step 6, arriving later and with a running service on top of it.
Then rebuild both halves exactly as in step 8 — **the frontend must be rebuilt, not just
restarted**, whenever `NEXT_PUBLIC_FRIDGE_API_URL` or any frontend code changes — re-run the
preflight, and `sudo systemctl restart personal-website-backend personal-website-frontend`.

Migrations run at boot inside `db::init_pool`. **An applied migration is immutable**: sqlx
compares checksums and refuses to start with `migration N was previously applied but has been
modified`. If that appears after a pull, the fix is a new migration, never an edit.

## Rollback

```bash
sudo -u personal-website git -C /opt/personal-website checkout <previous-sha>
```
then rebuild and restart. Note that this leaves the checkout in **detached HEAD**, so the next
`Redeploy` step's `git pull` will refuse until you check the branch back out — which is the
correct behaviour and worth expecting rather than debugging.

**Rolling back code does not roll back a migration**, and this project has no down-migrations. If the bad deploy added one, restore the database from the
backup taken before the deploy (step 10) — which is why that gate exists.

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

**The backend stops retrying after five failed starts in a minute**, and stays down until told
otherwise. `Start request repeated too quickly` means the limit tripped, and it refuses a manual
restart too, even after you have fixed the cause. Read the journal, fix what it names, then:

```bash
sudo systemctl reset-failed personal-website-backend
sudo systemctl restart personal-website-backend
```

Reachable the way it should be, and no other way. Both processes should be on `127.0.0.1`, and
the second command, run from **another machine**, should time out or be refused:

```bash
sudo ss -ltnp | grep -E ':(3000|8080) '
curl -m 5 http://YOUR-SERVER-IP:8080/health
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

## Required code changes — all done as of 2026-11-06

**Nothing in this section blocks the deploy any more.** It is kept struck through rather than
deleted: each row is a defect that only became visible once someone wrote down what deploying
actually involves, which is the argument for writing the runbook before the deploy rather than
during it. The Gmail link, the listening addresses and the restart limit were found by checking
this runbook against the code two months later, which is the argument for doing that before
following it. The rest were found by 10h and fixed by the sessions after it.

| What | Where | Why it matters |
|---|---|---|
| ~~The Gmail link went to `localhost`~~ | `frontend/src/app/internships/InboxPanel.tsx`, `lib/authApi.ts` | **Done 2026-11-05.** Connect and Reconnect were a literal `http://localhost:8080/auth/gmail/start`, so on a deployed site the click went to the visitor's own machine. Now `gmailConnectUrl()`, through `apiBase()` like every other backend URL |
| ~~Both processes listened on every interface~~ | `src/main.rs`, `deploy/personal-website-frontend.service`, `deploy/preflight.sh` | **Done 2026-11-05.** The backend hardcoded `0.0.0.0`, serving the API on plain HTTP at `:8080` past the proxy. The frontend unit set `HOSTNAME`, which `next start` does not read, so it did the same at `:3000` while its comment said loopback. Now `BIND_ADDRESS=127.0.0.1` (required by the preflight; the default stays every interface, for LAN development) and `--hostname 127.0.0.1` |
| ~~The restart limit could never trip~~ | `deploy/personal-website-backend.service` | **Done 2026-11-06.** `StartLimitBurst` and `StartLimitIntervalSec` sat under `[Service]`. systemd's parser still accepts the burst there, as a legacy alias, but has no `[Service]` entry for `StartLimitIntervalSec`, so the 60 seconds was ignored and the default interval applied. Five starts cannot fit in that window at `RestartSec=5s`, so a broken config would have restarted forever while the comment above it said otherwise. Both moved to `[Unit]`. |
| ~~Visitors were told about the backend's port~~ | `frontend/src/lib/apiClient.ts`, the blog, fridge and internship pages | **Done 2026-11-06.** With the API down, six pages said "Is the backend running on :8080?" and the public listing showed the browser's own "Failed to fetch". `unavailableMessage` keeps the port hint for `next dev` only. |
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

**Corrected 2026-11-05: pinning takes a code change.** This section recommended pinning the UUID
through `ALLOWED_ORIGINS`, but `is_allowed_origin` admits any `moz-extension://` origin whatever
that list says, so adding one narrows nothing. It also overstates the cost:
`apps/hunt-extension/CLAUDE.md` records that Firefox does not send the `SameSite=Lax` session
cookie from a `moz-extension://` page, which is why the extension has a bearer token at all.

**Recommended:** leave it for the first deploy. If it ever needs closing, the change is to make
the scheme check opt-in beside `ALLOWED_ORIGINS`. Putting the whole site behind a VPN would make
the argument moot, but the listing is now public.

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

**In the database, not in `backend.env`** (`gmail_accounts.refresh_token`; this section said
otherwise until 2026-11-05), so it is also in every backup of that database. `backend.env` holds
the client secret. The database and each backup are mode 600 and the backup directory is mode 700,
with `ProtectHome` and `ProtectSystem=strict` in the unit. The token grants `gmail.modify` on the
burner account.

**Recommended:** keep it the burner, never a primary mailbox. If the host is ever suspected,
revoking the app in the Google account is faster and more complete than replacing the token.

### Google's limits on Gmail

Two limits come from Google rather than this code. Check its current rules before relying on
either, because neither has been exercised here.

- **While the OAuth app is in Testing, Google expires the connection after 7 days**, so on a host
  nobody sits at, sync stops weekly until someone reconnects. Moving the consent screen to **In
  production** should end that, at the cost of Google's "unverified app" warning when connecting.
- **`gmail.modify` is a restricted scope.** Sign-in and the listing work for anyone, but another
  person connecting their inbox has to be added as a test user, or the app has to pass Google's
  verification. For now email sync is the owner's alone, which fits the panel presenting it as
  optional.

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

## Rehearsed locally, 2026-09-03 — six steps of eleven

Everything below was run on a development Mac against the real artifacts in `deploy/`. It is
not a substitute for the host, and the second list says exactly what it could not touch.

| Step | Result |
|---|---|
| `cargo build --release` | **Passes.** 9.2 s compile, 17 MB binary. It had never been run against this tree — the previous release binary predated migrations 0026–0031 and 25 other changed files |
| Frontend production build | **Passes**, `npm ci` and `next build` exactly as step 7 writes them. `NEXT_PUBLIC_FRIDGE_API_URL` is genuinely compiled into the client bundle — grepped for and found, not assumed |
| Restore drill | **Passes.** Byte-faithful against the backup it came from, and every table added since the drill was last recorded is present: `application_events`, `application_deadlines`, `resume_variants`, `source_run_scopes` |
| Preflight, *succeeding* | **Passes** — `looks deployable`, exit 0. Every previous run of it had been a run that refused, so its success condition had never actually been observed |
| Unit file paths | **6 of 6 resolve** against this repo at the same relative path, so the only variable on the host is the `/opt/personal-website` prefix |
| Caddyfile routing | **Correct.** `handle_path /api/*` strips the prefix, which is what `apiClient::apiBase()` expects when the env var points at `…/api`. Syntax **not** validated — `caddy` is not installed here |

### Two stale claims it found, both in what you would read before going live

**`deploy/Caddyfile` said the rate limiter shares one IP bucket behind the proxy**, and that its
`header_up` lines were "set for the day the backend learns to trust one hop". That day was
2026-09-02. `rate_limit.rs:client_ip` trusts the last hop when the TCP peer is loopback, and
`a_forwarded_header_is_ignored_from_a_remote_peer` pins the other half. The comment described a
live denial-of-service vector that no longer exists, in the file an operator reads while
installing the proxy.

**`rate_limit.rs`'s own module header said the same thing**, thirty lines above the function
documenting at length why and how the header *is* trusted. One file, contradicting itself.

Both corrected. Had they been found on the host instead, the likely cost was not an outage but
an hour spent working around a problem that had already been fixed — or, worse, deleting the two
`header_up` lines as pointless, which would have *created* the vector the comment warned about.

### Rehearsed again, 2026-11-05 — the two listening addresses and the Gmail link

A production build from a scratch copy of `frontend/`, with the API URL baked in as step 8 does,
run as the units run it, against a throwaway database with every background worker off.

| Check | Result |
|---|---|
| Backend with `BIND_ADDRESS=127.0.0.1` | Listens on `127.0.0.1`; connecting by the machine's LAN address is refused |
| Frontend with `--hostname 127.0.0.1` | Listens on `127.0.0.1`; connecting by the LAN address is refused |
| **Control: the old `HOSTNAME=127.0.0.1`** | Listens on `*`, and answers **200 on the LAN address**. The old unit really was exposed |
| Backend with `BIND_ADDRESS` unset | Listens on `*:8080`, so LAN development is unchanged |
| `BIND_ADDRESS=localhost` | Exits 1 with the reason; nothing listens |
| Preflight on a filled-in copy of the example | Passes. Refuses `BIND_ADDRESS` unset, `0.0.0.0` and `::1` |
| The link, signed in on the production build | Renders the configured API URL plus `/auth/gmail/start`. `localhost:8080` appears nowhere in the client bundle, and every API call on the signed-in page went to the configured URL |

### Rehearsed again, 2026-11-06 — what a visitor sees when the API is down

A production build from a scratch copy with the API URL pointed at a closed port, then `next dev`
against a stub answering 500. The fridge pages were reached with a made-up session cookie, which
is all `proxy.ts` checks, on a throwaway hostname.

| Check | Result |
|---|---|
| Production: `/blog`, `/blog/<slug>`, `/internships`, and the fridge, shopping list, recipes and reviews pages | Each says "Couldn't load … Please try again in a moment." |
| The production client bundle | "Is the backend running" appears in no file |
| `next dev`, `/blog` | Keeps the hint: "Couldn't load the blog. Is the backend running on :8080?" |
| `/internships` when the API answers 500 | Keeps the client's own message: "Could not load internships (500)" |
| **Control: the browser's error for a closed port** | A `TypeError` reading "Failed to fetch", which is what the listing used to show |

### What could not be rehearsed here, and is still genuinely untested

- **systemd** does not run on macOS. The units are path-checked and never started.
  The restart-limit fix was checked against systemd's manual and its parser table, not seen
  tripping. Step 11's `systemd-analyze verify` is the first real check, and the first broken start
  is the first real test.
- **TLS and DNS**: Caddy's certificate provisioning needs the real domain to resolve.
- **The Google OAuth round trip**, both redirect URIs.
- **Build time and memory on a small box.** The 9.2 s above is an incremental compile on a
  developer machine with every dependency already built and a warm npm cache. It says the code
  compiles in release mode; it says **nothing** about the cold first build on a 2 GB VPS, and
  the estimate in "Before you start" is still an estimate.
- **The restore drill on the host**, with the backend stopped, which is step 10 and remains the
  gate.
- **The firewall, toolchain, build-`HOME` and data-move commands in steps 4–9.** Written on
  2026-11-05 and never run on Linux. They are the likeliest place for the first snag.
- **Path stripping through Caddy.** The 2026-11-05 rehearsal pointed the API URL straight at the
  backend, because `caddy` is not installed here, so `/api/…` end to end is first exercised on the
  host.

---

## The `[you]` half of 10h, in order

Nothing below can be done by an agent: it needs an account, a card, DNS, or a decision.

1. ~~Gate — merge `phase-10-hardening`~~ — done.
2. **Gate — drill the restore on the host** (10i). The scripts and a dev-machine drill exist;
   what is still yours is one restore performed on the deployed host, before the first start
   (step 10 of **First deploy**).
3. ~~Decide **`INBOX_APPLY_LABELS`**~~ — decided 2026-09-03: **ON**, and the example env file
   ships `true`. Nothing to do but keep the explicit value when you fill in the real file.
4. ~~Decide the **`moz-extension://`** posture~~ — nothing to decide for the first deploy. Pinning
   takes a code change; see § The `moz-extension://` wildcard.
5. Buy/choose the host and point DNS at it. Caddy provisions the certificate on first start,
   which requires the DNS record to already resolve.
6. Register **both** redirect URIs on your Google Cloud OAuth client, and decide about **In
   production** (§ Google's limits on Gmail).
7. Fill `/etc/personal-website/backend.env` from `deploy/env.production.example` and run the preflight.
8. Deploy per **First deploy**, then confirm **Is it alive?** — including the company-tier line.
9. **Migration `0025` merges rows on the first boot of any database behind it**, which the laptop's
   is not. It re-keys 413 postings and merges 58 duplicate groups, repointing any application that
   pointed at a row it deletes. It was verified against a copy (1,828 → 1,764 postings, zero
   orphans), and it is guarded so that a repoint it cannot make leaves the duplicate in place
   rather than failing — but it is still the first migration in this project that deletes
   user-visible rows. **On such a database the backup in step 10 is not optional before this
   boot**, and `ops/backup-fridge-db.sh` is how.
10. **Check the event history before looking at any analytics.** A database brought from the
   laptop already has it, so step 9 of **First deploy** only verifies it. On any database where
   `verify` reports nothing covered, run the backfill once, with `DATABASE_URL` spelled out:

   ```bash
   sudo -u personal-website env DATABASE_URL='sqlite:///var/lib/personal-website/fridge.db?mode=rwc' /opt/personal-website/apps/fridge-app/backend/target/release/fridge_backend application-events backfill
   ```

   Without it the dashboard is not empty — it is *wrong*. `applications` is counted from the
   applications table while every conversion number comes from `application_events`, so an
   un-backfilled database shows a populated funnel in which nothing has ever responded.
   Measured on a real copy during Checkpoint 11: `responded 0 → 1`, `reached_oa 0 → 1`, with no
   indication in the response that the log had been empty.
11. Update the extension: set its Settings backend URL to `https://hunt.example.com/api`, its site
   URL to `https://hunt.example.com`, and press **Test connection**, which is what triggers
   Firefox's permission prompt for that origin — accept it. No manifest edit and no reload: the
   grant is per origin and happens at runtime. `ALLOWED_ORIGINS` needs no entry for the extension,
   because the backend admits any `moz-extension://` origin.
12. Keep the laptop's backend off, and watch the host for 48 hours without touching it. That is
    Checkpoint 10, and it is also when the corpus Phase 13 needs begins to accumulate.
