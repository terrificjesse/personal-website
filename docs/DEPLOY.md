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

### The build path is load-bearing

`internships::prestige::CompanyTiers::load()` resolves its data file against
`env!("CARGO_MANIFEST_DIR")` — **the path the binary was built in, compiled into the binary**.
A `releases/<date>/` layout that prunes old releases leaves that path dangling. The tier file
then fails to load, prestige silently degrades to derived-only, and the tier-1/2 alert
predicate stops firing.

The symptom is *no notifications*, which is indistinguishable from a quiet job market, and the
only evidence is one log line at startup. So: **build in place at a stable path**, and check the
line after every deploy:

```bash
journalctl -u personal-website-backend --since "5 min ago" | grep -i "company tier"
```

Silence there is good; `no company tier file at …` means alerting is degraded. The durable fix
is a required change, listed below.

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
   unparseable sync interval, and an unset `INBOX_APPLY_LABELS` (task 10k — see below).
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

## Required code changes — named here because 10h does not own these files

Each blocks or degrades something after deploy. None is written by this task.

| What | Where | Why it matters |
|---|---|---|
| **The extension cannot reach a deployed backend.** `host_permissions` are exactly `localhost:8080` and `127.0.0.1:8080` | `apps/hunt-extension/manifest.json` | After deploy the extension reports `unpermitted` — Firefox MV3 grants host permissions per origin and the manifest never names the new one. Add the public origin, then re-grant via **Test connection**. Until then: no desktop alerts at all |
| **The rate limiter cannot see the caller behind a proxy** | `apps/fridge-app/backend/src/rate_limit.rs` (10j, Codex) | See the risk table below. This is the one that can lock the user out |
| **The tier file path is compiled in** | `src/internships/prestige.rs:97` | Resolve it from the executable's own directory or an env var instead of `CARGO_MANIFEST_DIR`, so the deployed layout stops being load-bearing |
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

### `INBOX_APPLY_LABELS` defaults to ON — this is task 10k

`labelling_enabled()` returns true unless the variable is `false`/`0`. The code's reasoning is
recorded and is not unreasonable: the `gmail.modify` scope withholds permanent delete and send,
`labels.rs` never removes a label, never archives, and never touches a disregarded message. But
`docs/PLAN.md` § Phase 9 said label writes were *held* until 8b met a real corpus, and 8b's
checkpoint is still unmet.

**Recommended: `INBOX_APPLY_LABELS=false` for the first fortnight.** That is exactly the window
Phase 13 needs the mail for, and classification does not need write access to be measured — the
verdicts are stored either way. Turn it on after 13c grades the corpus. The preflight refuses to
start without an explicit value, so this gets decided rather than inherited.

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

**Recommended, in order:** (1) teach the limiter to trust `X-Forwarded-For` **only** when the
peer is loopback, taking the last hop — the proxy is the only thing that can reach the backend,
so the header is trustworthy exactly there; (2) until then, raise the login IP bucket well above
the account bucket so it stops being the binding constraint; (3) do not simply drop the IP
bucket — it is what stops one client spraying account names. The Caddyfile already sets both
forwarded headers so (1) is a backend change alone.

---

## The `[you]` half of 10h, in order

Nothing below can be done by an agent: it needs an account, a card, DNS, or a decision.

1. ~~Gate — merge `phase-10-hardening`~~ — done.
2. **Gate — drill the restore on the host** (10i). The scripts and a dev-machine drill exist;
   what is still yours is one restore performed on the deployed host, with the backend stopped.
3. Decide **`INBOX_APPLY_LABELS`** (task 10k). Recommended `false` for the first fortnight.
4. Decide the **`moz-extension://`** posture: pin the UUID, or put the host behind a VPN.
5. Buy/choose the host and point DNS at it. Caddy provisions the certificate on first start,
   which requires the DNS record to already resolve.
6. Create the Google Cloud OAuth client and register **both** redirect URIs.
7. Fill `/etc/personal-website/backend.env` from `deploy/env.production.example` and run the preflight.
8. Deploy per **First deploy**, then confirm **Is it alive?** — including the company-tier line.
9. Update the extension: the manifest's host permissions (a code change, listed above), its
   Settings backend URL (`https://hunt.example.com/api`), and re-grant via **Test connection**.
10. Watch it for 48 hours without touching it. That is Checkpoint 10, and it is also when the
    corpus Phase 13 needs begins to accumulate.
