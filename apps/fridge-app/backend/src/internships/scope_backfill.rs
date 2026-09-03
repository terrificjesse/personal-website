//! Task 12k's measurement and generator for migration `0028`, which gives the Greenhouse
//! sightings that predate `0026` the scope tag they would have got had they been seen since.
//!
//! # Why this is a generator rather than SQL
//!
//! The board slug is in `posting_sightings.url`, and `dedup::ats_identity` already knows how to
//! read it — including that Greenhouse serves the same boards from `boards.greenhouse.io`,
//! `job-boards.greenhouse.io` and a regional host, and that this one host's path may be
//! case-folded. Re-implementing that in SQLite string functions would agree with it right up
//! until it didn't, and the failure mode of the divergence is a sighting tagged to a board that
//! does not exist, which then never advances again. So the migration is emitted from the same
//! parser the rest of the pipeline uses.
//!
//! # Invocation
//!
//! Read-only, against a copy — never `fridge.db`. See `docs/INTERNSHIP_SCRAPING.md` § D.4.
//!
//! ```text
//! sqlite3 fridge.db ".backup '/tmp/scope-backfill.db'"
//! SCOPE_FIXTURE_DB=/tmp/scope-backfill.db \
//!   cargo test -p fridge_backend scope_backfill -- --ignored --nocapture
//! ```
//!
//! Set `SCOPE_BACKFILL_OUT` as well to write the migration body to that path.

#[cfg(test)]
mod tests {
    use super::super::dedup::ats_identity;
    use super::super::sources::BoardDirectory;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        source: String,
        url: String,
        scope: Option<String>,
    }

    /// Why a sighting was or was not given a tag. Every row lands in exactly one, so the
    /// report's buckets sum to the row count and a silently dropped row is impossible.
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    enum Verdict {
        /// Taggable: the URL yields a slug the directory actually polls.
        Taggable(String),
        /// A real run already tagged it. A live tag beats a derived one, always.
        AlreadyTagged,
        /// The URL is not on a recognised ATS at all.
        Unparsed,
        /// Parsed, but as some other ATS — a Greenhouse-sourced row pointing elsewhere.
        OtherAts(String),
        /// `ats_identity` has two deliberate pseudo-slugs for Greenhouse jobs whose board is
        /// unknowable from the URL: `embed` (the embed form) and `gh_jid` (a job id carried in
        /// a query parameter by a company's own careers page). Neither is a board, neither is
        /// ever polled, and neither may become a scope — a sighting tagged with one would be
        /// waiting forever for a board that does not exist.
        PseudoSlug(String),
        /// A slug the board directory does not contain, so nothing ever polls it. Tagging with
        /// it would make the row permanently unable to advance, where untagged it can still
        /// advance on a fully successful run. Left alone.
        SlugNotPolled(String),
    }

    fn classify(row: &Row, directory: &BTreeSet<&str>) -> Verdict {
        if row.scope.is_some() {
            return Verdict::AlreadyTagged;
        }
        let Some(identity) = ats_identity(&row.url) else {
            return Verdict::Unparsed;
        };
        if identity.ats != "greenhouse" {
            return Verdict::OtherAts(identity.ats);
        }
        if matches!(identity.board_slug.as_str(), "embed" | "gh_jid") {
            return Verdict::PseudoSlug(identity.board_slug);
        }
        if !directory.contains(identity.board_slug.as_str()) {
            return Verdict::SlugNotPolled(identity.board_slug);
        }
        Verdict::Taggable(identity.board_slug)
    }

    // ---- the shape of the generated file, which is what review actually rests on ----

    const MIGRATION: &str = include_str!("../../migrations/0028_backfill_greenhouse_scopes.sql");

    fn statements() -> Vec<&'static str> {
        MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| statement.starts_with("UPDATE"))
            .collect()
    }

    #[test]
    fn every_statement_is_scoped_to_greenhouse_and_to_untagged_rows() {
        let statements = statements();
        assert!(!statements.is_empty(), "0028 must contain UPDATE statements");
        for statement in &statements {
            assert!(
                statement.contains("source = 'greenhouse'"),
                "a scope on an unscoped source is a tag no run ever completes:\n{statement}"
            );
            assert!(
                statement.contains("scope IS NULL"),
                "a tag written by a real run must always beat a derived one:\n{statement}"
            );
        }
    }

    #[test]
    fn no_statement_tags_a_pseudo_slug() {
        // `embed` and `gh_jid` are `ats_identity`'s stand-ins for "the URL does not name a
        // board". Neither is ever polled, so a sighting tagged with one waits forever.
        for statement in statements() {
            assert!(
                !statement.contains("scope = 'embed'") && !statement.contains("scope = 'gh_jid'"),
                "pseudo-slugs must never become scopes:\n{statement}"
            );
        }
    }

    #[test]
    fn every_slug_named_is_one_the_board_directory_actually_polls() {
        // The failure this prevents: a tag pointing at a board nothing fetches, which converts
        // "advances on a fully successful run" into "never advances again".
        let vendored = BoardDirectory::vendored();
        let known: BTreeSet<&str> = vendored
            .slugs("greenhouse")
            .iter()
            .map(String::as_str)
            .collect();
        for statement in statements() {
            let slug = statement
                .split_once("scope = '")
                .and_then(|(_, rest)| rest.split_once('\''))
                .map(|(slug, _)| slug)
                .expect("every statement sets a scope");
            assert!(
                known.contains(slug),
                "0028 names `{slug}`, which the board directory does not carry"
            );
        }
    }

    #[tokio::test]
    #[ignore = "12k runs this against a copy named by SCOPE_FIXTURE_DB"]
    async fn report_and_generate_the_scope_backfill() {
        let fixture = PathBuf::from(
            std::env::var("SCOPE_FIXTURE_DB")
                .expect("set SCOPE_FIXTURE_DB to a copy of the database"),
        );
        assert_ne!(
            fixture.file_name().and_then(|name| name.to_str()),
            Some("fridge.db"),
            "refusing the live database path; make the copy first"
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&fixture).read_only(true))
            .await
            .expect("open the fixture read-only");

        let rows = sqlx::query_as::<_, Row>(
            "SELECT id, source, url, scope FROM posting_sightings ORDER BY source, external_id",
        )
        .fetch_all(&pool)
        .await
        .expect("read sightings");

        let vendored = BoardDirectory::vendored();
        let directory: BTreeSet<&str> = vendored
            .slugs("greenhouse")
            .iter()
            .map(String::as_str)
            .collect();

        // Greenhouse is the only scoped source. Everything else is counted and never touched,
        // because a tag on an unscoped source's sighting would be a scope no run ever completes.
        let (greenhouse, others): (Vec<&Row>, Vec<&Row>) =
            rows.iter().partition(|row| row.source == "greenhouse");

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_slug: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for row in &greenhouse {
            let verdict = classify(row, &directory);
            let bucket = match &verdict {
                Verdict::Taggable(slug) => {
                    by_slug.entry(slug.clone()).or_default().push(&row.id);
                    "taggable".to_string()
                }
                Verdict::AlreadyTagged => "already tagged by a real run".to_string(),
                Verdict::Unparsed => "url is not on a recognised ATS".to_string(),
                Verdict::OtherAts(ats) => format!("parsed as another ATS: {ats}"),
                Verdict::PseudoSlug(slug) => {
                    format!("pseudo-slug `{slug}`: the URL does not name a board")
                }
                Verdict::SlugNotPolled(_) => "slug absent from the board directory".to_string(),
            };
            *counts.entry(bucket).or_default() += 1;
        }

        println!("\n=== 12k: scope backfill, {} greenhouse sightings ===", greenhouse.len());
        for (bucket, count) in &counts {
            println!("  {count:>4}  {bucket}");
        }
        println!("  ---- {} boards would be named across {} taggable rows",
                 by_slug.len(), by_slug.values().map(Vec::len).sum::<usize>());

        let other_ats: usize = others
            .iter()
            .filter(|row| ats_identity(&row.url).is_some())
            .count();
        println!(
            "\n  {} sightings on unscoped sources ({} of them ATS-shaped) — none are touched",
            others.len(),
            other_ats
        );

        // Slugs the directory does not carry, named rather than merely counted: each one is a
        // row this backfill declines to help, and the list is how anyone decides whether to
        // harvest the slug instead.
        let unpolled: BTreeSet<String> = greenhouse
            .iter()
            .filter_map(|row| match classify(row, &directory) {
                Verdict::SlugNotPolled(slug) => Some(slug),
                _ => None,
            })
            .collect();
        if !unpolled.is_empty() {
            println!("  unpolled slugs: {:?}", unpolled);
        }

        if let Ok(out) = std::env::var("SCOPE_BACKFILL_OUT") {
            let mut sql = String::new();
            for (slug, ids) in &by_slug {
                assert!(
                    !slug.contains('\'') && !slug.is_empty(),
                    "a slug that needs quoting has no business being a board name: {slug:?}"
                );
                let list = ids
                    .iter()
                    .map(|id| format!("'{id}'"))
                    .collect::<Vec<_>>()
                    .join(", ");
                sql.push_str(&format!(
                    "UPDATE posting_sightings SET scope = '{slug}'\n WHERE source = 'greenhouse' \
                     AND scope IS NULL AND id IN ({list});\n"
                ));
            }
            std::fs::write(&out, sql).expect("write the generated migration body");
            println!("\n  wrote {} statements to {out}", by_slug.len());
        }
    }
}
