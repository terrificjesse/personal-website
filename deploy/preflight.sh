#!/usr/bin/env bash
# Refuse to start on a misconfiguration, rather than booting into one.
#
# Phase 10h. Runs as the backend unit's ExecStartPre, so a bad environment is a failed start
# with a reason in the journal — not a service that comes up looking healthy and behaves
# wrongly. Every check here corresponds to a failure this project has actually had, or to a
# default that is safe on a laptop and wrong on a public host.
#
#   ./deploy/preflight.sh /etc/hunt/backend.env
#
# Exits 0 when it is safe to start, 1 with one line per problem otherwise.

set -uo pipefail

# Fail CLOSED. A check script that dies halfway and still exits 0 is worse than no check at
# all: it reports "deployable" for a configuration nobody finished reading. (It did exactly
# that once — `declare -A` is a bash 4 feature and the host running this may be bash 3.2.)
set -E
trap 'echo "preflight: internal error at line $LINENO — refusing to vouch for this config" >&2; exit 1' ERR

env_file="${1:-/etc/hunt/backend.env}"
problems=()

if [[ ! -r "$env_file" ]]; then
  echo "preflight: cannot read $env_file" >&2
  exit 1
fi

# Read values without executing the file: it holds secrets and may contain characters a shell
# would interpret. Commented lines start with `#`, so they never match. Last assignment wins,
# which is how systemd's EnvironmentFile behaves.
get() {
  sed -n "s/^[[:space:]]*$1=//p" "$env_file" \
    | tail -n 1 \
    | tr -d '\r' \
    | sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\(.*\)'$/\1/"
}
set_and_nonempty() { [[ -n "$(get "$1")" ]]; }

# --- 1. cookies -----------------------------------------------------------------------------
# COOKIE_SECURE defaults OFF so the app works over plain HTTP on a LAN. Behind HTTPS that
# default means session cookies travel in the clear.
case "$(get COOKIE_SECURE)" in
  true|1) ;;
  *) problems+=("COOKIE_SECURE must be true behind HTTPS (found: '$(get COOKIE_SECURE)')") ;;
esac

# --- 2. credentialed CORS -------------------------------------------------------------------
# A wildcard is rejected by browsers on credentialed requests, and the failure is silent —
# every call just looks signed-out.
if ! set_and_nonempty ALLOWED_ORIGINS; then
  problems+=("ALLOWED_ORIGINS is unset; credentialed requests will be discarded by the browser")
elif [[ "$(get ALLOWED_ORIGINS)" == *"*"* ]]; then
  problems+=("ALLOWED_ORIGINS contains '*', which browsers reject on credentialed requests")
elif [[ "$(get ALLOWED_ORIGINS)" == *"localhost"* || "$(get ALLOWED_ORIGINS)" == *"127.0.0.1"* ]]; then
  problems+=("ALLOWED_ORIGINS still names localhost — that is the dev value, not the deployed origin")
fi

if ! set_and_nonempty FRONTEND_ORIGIN; then
  problems+=("FRONTEND_ORIGIN is unset; the OAuth round trip has nowhere to return to")
elif [[ "$(get FRONTEND_ORIGIN)" != https://* ]]; then
  problems+=("FRONTEND_ORIGIN is not https:// — a Secure cookie will never be sent")
fi

# --- 3. the database ------------------------------------------------------------------------
# A relative path resolves against WorkingDirectory, so a deploy that moves the checkout
# silently creates a second, empty database. An absolute path outside every checkout is the
# only version that survives a redeploy and is visible to the backups.
db_url="$(get DATABASE_URL)"
if [[ -z "$db_url" ]]; then
  problems+=("DATABASE_URL is unset")
else
  db_path="${db_url#sqlite://}"
  db_path="${db_path%%\?*}"
  if [[ "$db_path" != /* ]]; then
    problems+=("DATABASE_URL is a relative path ('$db_path'); it must be absolute")
  else
    case "$db_path" in
      */current/*|*/releases/*|*/repo/*|*/personal-website/*)
        problems+=("DATABASE_URL ('$db_path') is inside a checkout; a redeploy would orphan it") ;;
    esac
    db_dir="$(dirname "$db_path")"
    [[ -d "$db_dir" ]] || problems+=("DATABASE_URL directory $db_dir does not exist")
    [[ -w "$db_dir" ]] || problems+=("DATABASE_URL directory $db_dir is not writable by $(id -un)")
  fi
fi

# --- 4. Google, all or nothing --------------------------------------------------------------
# Half-configured is the failure that reports "Google OAuth not configured" at boot and then
# looks like a missing route at the callback.
google_present=0
google_absent=0
for key in GOOGLE_CLIENT_ID GOOGLE_CLIENT_SECRET GOOGLE_REDIRECT_URI; do
  if set_and_nonempty "$key"; then google_present=$((google_present + 1)); else google_absent=$((google_absent + 1)); fi
done
if (( google_present > 0 && google_absent > 0 )); then
  problems+=("Google OAuth is half-configured: $google_present of 3 set. Set all three or none")
fi

# --- 5. the inbox agent ---------------------------------------------------------------------
# UNSET IS NOT OFF: the interval defaults to 900s, and labelling defaults ON. Both are fine on
# a laptop and are decisions on a host that runs unattended against a real mailbox (task 10k).
interval="$(get INBOX_SYNC_INTERVAL_SECS)"
if [[ -n "$interval" && ! "$interval" =~ ^[0-9]+$ ]]; then
  problems+=("INBOX_SYNC_INTERVAL_SECS='$interval' is not a number; the agent would disable itself and log, which reads as a Gmail outage")
fi
if [[ "$interval" != "0" ]]; then
  if (( google_present < 3 )); then
    problems+=("the inbox sync is enabled (INBOX_SYNC_INTERVAL_SECS='${interval:-unset -> 900}') but Google OAuth is not fully configured")
  fi
  set_and_nonempty GMAIL_REDIRECT_URI || problems+=("GMAIL_REDIRECT_URI is unset while the inbox sync is enabled")
  case "$(get INBOX_APPLY_LABELS)" in
    true|false|0|1) ;;
    *) problems+=("INBOX_APPLY_LABELS must be set explicitly (it defaults to ON) — this is task 10k") ;;
  esac
fi

threshold="$(get INBOX_AUTO_APPLY_CONFIDENCE)"
if [[ -n "$threshold" ]]; then
  echo "preflight: NOTE — INBOX_AUTO_APPLY_CONFIDENCE=$threshold; email-driven status changes will apply themselves above it." >&2
fi

# --- report ----------------------------------------------------------------------------------
if (( ${#problems[@]} > 0 )); then
  echo "preflight: refusing to start, ${#problems[@]} problem(s) in $env_file:" >&2
  for problem in "${problems[@]}"; do echo "  - $problem" >&2; done
  exit 1
fi

echo "preflight: $env_file looks deployable"
