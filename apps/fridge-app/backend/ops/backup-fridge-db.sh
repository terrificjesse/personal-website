#!/usr/bin/env bash
# Online backup for fridge.db (Phase 10i).
#
# Contract:
#   - FRIDGE_DB_PATH names the live SQLite file.
#   - FRIDGE_BACKUP_DIR is a dedicated directory, never the repository.
#   - Each run uses SQLite's online backup API, verifies PRAGMA integrity_check and the sqlx
#     migration ledger, then publishes the snapshot with an atomic rename.
#   - FRIDGE_BACKUP_RETENTION_DAYS defaults to 30. Set it to 0 to retain every backup.
#   - Output is the final backup path, suitable for the systemd journal or another script.
#   - Backups contain the plaintext Gmail refresh token carried by fridge.db. The directory is
#     forced to mode 0700 and each backup to 0600; any remote volume must also encrypt at rest.
#
# A plain `cp` is deliberately not used: copying a live database can separate the main file
# from an active journal/WAL and produce a snapshot that was never a real database state.

set -euo pipefail
umask 077

usage() {
    printf '%s\n' \
        "Usage: FRIDGE_DB_PATH=/path/fridge.db FRIDGE_BACKUP_DIR=/path/backups $0" \
        "" \
        "Optional: FRIDGE_BACKUP_RETENTION_DAYS=30 (0 keeps every backup)." \
        "The script prints the new verified backup path on stdout." >&2
}

die() {
    echo "backup-fridge-db: $*" >&2
    exit 1
}

sqlite_backup() {
    local source_db=$1
    local destination_db=$2

    case "$destination_db" in
        *\"* | *$'\n'*) die "backup paths may not contain quotes or newlines" ;;
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

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    usage
    exit 0
fi
[[ $# -eq 0 ]] || { usage; exit 2; }

command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 is required"

source_db=${FRIDGE_DB_PATH:-}
backup_dir=${FRIDGE_BACKUP_DIR:-}
retention_days=${FRIDGE_BACKUP_RETENTION_DAYS:-30}

[[ -n "$source_db" ]] || die "FRIDGE_DB_PATH is required"
[[ -f "$source_db" && ! -L "$source_db" ]] || die "source is not a regular database file: $source_db"
[[ -n "$backup_dir" && "$backup_dir" != "/" && "$backup_dir" != "." ]] \
    || die "FRIDGE_BACKUP_DIR must be a dedicated directory"
[[ "$retention_days" =~ ^[0-9]+$ ]] \
    || die "FRIDGE_BACKUP_RETENTION_DAYS must be a non-negative integer"

mkdir -p "$backup_dir"
chmod 700 "$backup_dir"

timestamp=$(date -u +%Y%m%dT%H%M%SZ)
final_backup="$backup_dir/fridge-$timestamp.db"
[[ ! -e "$final_backup" ]] || die "a backup already exists for this second: $final_backup"

temporary_backup=$(mktemp "$backup_dir/.fridge-backup.XXXXXX")
cleanup() {
    if [[ -n "${temporary_backup:-}" ]]; then
        rm -f "$temporary_backup"
    fi
    return 0
}
trap cleanup EXIT

sqlite_backup "$source_db" "$temporary_backup"
verify_database "$temporary_backup"
chmod 600 "$temporary_backup"
mv "$temporary_backup" "$final_backup"
temporary_backup=

if [[ "$retention_days" -gt 0 ]]; then
    # The directory and filename prefix are both constrained above. Print deletions so the
    # systemd journal retains an audit trail of the retention half of the job. They go to
    # stderr so stdout remains the one-line final path promised by the module contract.
    find "$backup_dir" -maxdepth 1 -type f -name 'fridge-*.db' \
        -mtime "+$retention_days" -print -delete >&2
fi

echo "$final_backup"
