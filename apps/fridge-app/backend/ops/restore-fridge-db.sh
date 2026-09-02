#!/usr/bin/env bash
# Verified restore path for fridge.db (Phase 10i).
#
# Safe drill (does not overwrite anything):
#   restore-fridge-db.sh BACKUP /tmp/fridge-restored.db
#
# Real replacement:
#   1. Stop the backend and confirm it is no longer holding fridge.db open.
#   2. Run: restore-fridge-db.sh --replace BACKUP /var/lib/personal-website/fridge.db
#   3. Start the backend and check GET /health plus one authenticated read.
#
# Contract:
#   - BACKUP must pass PRAGMA integrity_check and contain a non-empty `_sqlx_migrations` ledger.
#   - The restored candidate is built and verified in the destination directory, then renamed
#     atomically over the target.
#   - An existing target is never touched without `--replace`. Replacement first creates and
#     verifies DESTINATION.pre-restore-<UTC timestamp>, so rollback remains possible.
#   - The backend must be stopped for replacement. Renaming an open SQLite file can leave the
#     running process attached to the old inode and its journal; `--replace` is the operator's
#     explicit confirmation that the stop step happened.
#
# Verified drill, 2026-09-02 UTC: the backup script snapshotted the live 11 MB fridge.db while
# the existing backend held it open. Backup and restore both reported `integrity_check = ok` and
# preserved 20 migration rows, 1 user, 2 applications, and 14 email messages. Both files were
# mode 0600 because they contain every live credential stored in fridge.db. This branch's
# backend then booted against the restored file, returned 200 from `/health`, and returned both
# applications through an authenticated request. `--replace` also refused while that restored
# database was open. The throwaway session and the temporary drill directory were removed.

set -euo pipefail
umask 077

usage() {
    printf '%s\n' \
        "Usage: $0 [--replace] BACKUP DESTINATION" \
        "" \
        "Safe drill (DESTINATION must not exist):" \
        "  $0 BACKUP /tmp/fridge-restored.db" \
        "" \
        "Replace the live database:" \
        "  1. Stop the backend and confirm it released fridge.db." \
        "  2. $0 --replace BACKUP /var/lib/personal-website/fridge.db" \
        "  3. Start the backend; check GET /health and one authenticated read." \
        "" \
        "Replacement saves the old DB beside the target as a verified .pre-restore file." >&2
}

die() {
    echo "restore-fridge-db: $*" >&2
    exit 1
}

sqlite_backup() {
    local source_db=$1
    local destination_db=$2

    case "$destination_db" in
        *\"* | *$'\n'*) die "restore paths may not contain quotes or newlines" ;;
    esac

    sqlite3 "$source_db" ".timeout 5000" ".backup \"$destination_db\""
}

verify_database() {
    local database=$1
    local integrity
    local migrations

    integrity=$(sqlite3 "$database" "PRAGMA integrity_check;") \
        || die "could not run integrity_check on $database"
    [[ "$integrity" == "ok" ]] \
        || die "integrity_check failed for $database: $integrity"

    migrations=$(sqlite3 "$database" "SELECT COUNT(*) FROM _sqlx_migrations;") \
        || die "$database has no readable sqlx migration ledger"
    [[ "$migrations" =~ ^[0-9]+$ && "$migrations" -gt 0 ]] \
        || die "$database has an empty or invalid sqlx migration ledger"
}

replace=0
if [[ "${1:-}" == "--replace" ]]; then
    replace=1
    shift
fi
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    usage
    exit 0
fi
[[ $# -eq 2 ]] || { usage; exit 2; }

command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 is required"

backup=$1
destination=$2
destination_dir=$(dirname "$destination")

[[ -f "$backup" && ! -L "$backup" ]] || die "backup is not a regular file: $backup"
[[ -d "$destination_dir" ]] || die "destination directory does not exist: $destination_dir"
[[ ! -L "$destination" ]] || die "destination may not be a symlink: $destination"
[[ ! -e "$destination" || ! "$backup" -ef "$destination" ]] \
    || die "backup and destination are the same file"
if [[ -e "$destination" && "$replace" -ne 1 ]]; then
    die "destination exists; stop the backend and pass --replace to overwrite it"
fi
if [[ -e "$destination" && ! -f "$destination" ]]; then
    die "destination exists but is not a regular file: $destination"
fi

verify_database "$backup"

restore_candidate=$(mktemp "$destination_dir/.fridge-restore.XXXXXX")
rollback_candidate=
cleanup() {
    if [[ -n "${restore_candidate:-}" ]]; then
        rm -f "$restore_candidate"
    fi
    if [[ -n "${rollback_candidate:-}" ]]; then
        rm -f "$rollback_candidate"
    fi
    return 0
}
trap cleanup EXIT

sqlite_backup "$backup" "$restore_candidate"
verify_database "$restore_candidate"
chmod 600 "$restore_candidate"

rollback=
if [[ -e "$destination" ]]; then
    if command -v lsof >/dev/null 2>&1 && lsof "$destination" >/dev/null 2>&1; then
        die "destination is open by a process; stop the backend before restoring"
    fi

    timestamp=$(date -u +%Y%m%dT%H%M%SZ)
    rollback="${destination}.pre-restore-${timestamp}"
    [[ ! -e "$rollback" ]] || die "rollback path already exists: $rollback"
    rollback_candidate=$(mktemp "$destination_dir/.fridge-rollback.XXXXXX")
    sqlite_backup "$destination" "$rollback_candidate"
    verify_database "$rollback_candidate"
    chmod 600 "$rollback_candidate"
    mv "$rollback_candidate" "$rollback"
    rollback_candidate=
fi

mv -f "$restore_candidate" "$destination"
restore_candidate=

echo "restored $destination from $backup"
if [[ -n "$rollback" ]]; then
    echo "previous database saved as $rollback"
fi
