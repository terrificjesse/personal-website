//! Company-name canonicalization: the one part of fuzzy matching that is a string problem.
//!
//! # Why this is a curated table and not a metric
//!
//! `docs/INTERNSHIP_SCRAPING.md` § C concluded that fuzzy company/title matching was both the
//! wrong shape for `src/nlp.rs` *and* out of scope as a learning area. The second half expired
//! on 2026-09-03 when the repo owner lifted it for this tab. The first half did not, and
//! measuring the real corpus added a third reason that is more interesting than either:
//!
//! **No trailing-token rule can decide these cases.** The corpus contains
//! `citadel` / `citadel securities` and `jump trading` / `jump trading group`. The first pair is
//! two different employers — a hedge fund and a market maker, separate hiring, separate roles.
//! The second is one company. Both differ by a single descriptive token, and no string metric,
//! edit distance, or suffix list separates them, because the distinction is knowledge about
//! companies rather than a property of the strings.
//!
//! So the split here is deliberate: [`propose`] is a **candidate generator** that finds pairs
//! worth a human's attention, and `data/internships/company-aliases.json` is the **decision**,
//! reviewed once and committed. The generator may be re-run whenever the corpus grows; its
//! output is a question, never an answer.
//!
//! `normalize::company_key` applies the table, which is what makes this affect two things at
//! once: the fallback dedup key, and how `company_signals` groups.
//!
//! Only [`canonical_company`] is a runtime path. The generator, the report and the migration
//! writer are `#[cfg(test)]`, because they are a tool run against a copy of the database rather
//! than anything the server does — the same shape as `scope_backfill`.
//!
//! # What this deliberately does not do
//!
//! **No title matching.** § C's objection stands untouched there: "Engineer" is a substring of
//! a large fraction of the corpus, and a title rule loose enough to merge "SWE Intern" with
//! "Software Engineer Intern" is loose enough to merge two genuinely different roles at the
//! same company. Over-merging destroys a posting; under-merging shows a duplicate.
//!
//! **No parent/subsidiary knowledge.** `Google` / `Alphabet` is not a string problem and this
//! does not pretend otherwise.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The committed decision. See the file's own `_provenance` field.
const VENDORED: &str = include_str!("../../data/internships/company-aliases.json");

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AliasTable {
    #[serde(default)]
    aliases: BTreeMap<String, String>,
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(default)]
    refused: Vec<Refusal>,
}

/// A candidate a human looked at and rejected, kept so the next regeneration does not propose
/// it again as though it were new. The reason is the part worth preserving.
///
/// Only the tests and the candidate report read these fields at present. They are deserialized
/// rather than skipped because the alternative is a `refused` list that nothing validates, and
/// an unvalidated list is one that can silently contradict `aliases`.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Deserialize)]
pub struct Refusal {
    pub candidate: String,
    pub against: String,
    pub reason: String,
}

impl AliasTable {
    /// The committed table, or an empty one if it will not parse.
    ///
    /// Deliberately not a panic, for the same reason `BoardDirectory::vendored` is not: a
    /// malformed data file must degrade this to "no company is an alias for any other" — which
    /// under-merges, the safe direction — rather than stop the process from starting and take
    /// the fridge app down with it.
    pub fn vendored() -> Self {
        match serde_json::from_str(VENDORED) {
            Ok(table) => table,
            Err(error) => {
                eprintln!(
                    "internships: data/internships/company-aliases.json did not parse ({error}); \
                     company keys will not be canonicalized"
                );
                AliasTable::default()
            }
        }
    }

    pub fn canonical<'a>(&'a self, key: &'a str) -> &'a str {
        self.aliases.get(key).map(String::as_str).unwrap_or(key)
    }

    #[cfg(test)]
    pub fn aliases(&self) -> &BTreeMap<String, String> {
        &self.aliases
    }

    #[cfg(test)]
    pub fn refused(&self) -> &[Refusal] {
        &self.refused
    }
}

/// The canonical form of an already-normalized company key.
///
/// Takes the key `normalize::company_key` would otherwise return — lowercased, legal suffixes
/// already stripped — and maps it through the committed table. An unknown key is returned
/// unchanged, which is the whole corpus except the twenty-one entries in that file.
pub fn canonical_company(key: &str) -> String {
    table().canonical(key).to_string()
}

fn table() -> &'static AliasTable {
    use std::sync::OnceLock;
    static TABLE: OnceLock<AliasTable> = OnceLock::new();
    TABLE.get_or_init(AliasTable::vendored)
}

/// One pair worth a human's attention: `longer` might be another name for `shorter`.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub shorter: String,
    pub longer: String,
    /// The tokens `longer` adds. This is the field a reviewer actually reads.
    pub added: String,
}

/// Propose candidate aliases from a corpus of company keys.
///
/// The rule is **strict token prefix**: `anduril` proposes `anduril industries`, because the
/// second is the first plus trailing tokens. Nothing else is proposed — no edit distance, no
/// substring, no acronym expansion — because every looser rule measured on this corpus produced
/// more false pairs than true ones, and a reviewer who stops reading is worse than a narrower
/// generator.
///
/// This proposes `citadel securities` against `citadel`. That is correct behaviour: it is a
/// candidate, and a human rejected it. See the `refused` list in the data file.
#[cfg(test)]
pub fn propose(keys: &[String]) -> Vec<Candidate> {
    let tokens: Vec<Vec<&str>> = keys.iter().map(|k| k.split(' ').collect()).collect();
    let mut out = Vec::new();

    for (i, short) in keys.iter().enumerate() {
        for (j, long) in keys.iter().enumerate() {
            if i == j || tokens[i].len() >= tokens[j].len() {
                continue;
            }
            if tokens[j][..tokens[i].len()] != tokens[i][..] {
                continue;
            }
            out.push(Candidate {
                shorter: short.clone(),
                longer: long.clone(),
                added: tokens[j][tokens[i].len()..].join(" "),
            });
        }
    }

    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_table_parses() {
        let table = AliasTable::vendored();
        assert!(
            !table.aliases().is_empty(),
            "an empty table means the file stopped parsing and canonicalization silently stopped"
        );
    }

    #[test]
    fn citadel_securities_is_never_merged_into_citadel() {
        // The case the whole design exists for. Two different employers separated by one
        // descriptive token, which no string rule can decide.
        assert_eq!(canonical_company("citadel securities"), "citadel securities");
        assert_eq!(canonical_company("citadel"), "citadel");
    }

    #[test]
    fn every_refusal_is_absent_from_the_alias_table() {
        // The two halves of the file must not contradict each other: a key cannot be both
        // rejected and applied.
        let table = AliasTable::vendored();
        for refusal in table.refused() {
            assert!(
                !table.aliases().contains_key(&refusal.candidate),
                "`{}` is in `refused` and in `aliases`",
                refusal.candidate
            );
        }
    }

    #[test]
    fn no_alias_points_at_another_alias() {
        // One hop only. A chain would make the result depend on iteration order, and a cycle
        // would make it depend on where you started.
        let table = AliasTable::vendored();
        for (alias, canonical) in table.aliases() {
            assert!(
                !table.aliases().contains_key(canonical),
                "`{alias}` -> `{canonical}`, which is itself an alias"
            );
        }
    }

    #[test]
    fn a_company_nobody_has_aliased_passes_through_untouched() {
        assert_eq!(canonical_company("some new startup"), "some new startup");
        assert_eq!(canonical_company(""), "");
    }

    #[test]
    fn the_reviewed_merges_apply() {
        assert_eq!(canonical_company("palantir technologies"), "palantir");
        assert_eq!(canonical_company("jump trading group"), "jump trading");
        assert_eq!(canonical_company("susquehanna international group sig"), "susquehanna");
    }

    #[test]
    fn propose_finds_a_trailing_token_pair_and_nothing_looser() {
        let keys: Vec<String> = ["anduril", "anduril industries", "andurilx", "citadel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let found = propose(&keys);
        assert_eq!(found.len(), 1, "only the token-prefix pair: {found:?}");
        assert_eq!(found[0].shorter, "anduril");
        assert_eq!(found[0].longer, "anduril industries");
        assert_eq!(found[0].added, "industries");
    }

    /// Task 12d's reproducible, read-only candidate report.
    ///
    /// Ignored because the fixture is a copy of the real database, not a committed one. The
    /// generator proposes; this prints what it proposed alongside what the committed file
    /// already says about each pair, so a review is a diff rather than a re-read.
    ///
    /// ```text
    /// sqlite3 fridge.db ".backup '/tmp/company-candidates.db'"
    /// COMPANY_FIXTURE_DB=/tmp/company-candidates.db \
    ///   cargo test -p fridge_backend company_match -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "12d runs this against a copy named by COMPANY_FIXTURE_DB"]
    async fn report_company_alias_candidates() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        let fixture = PathBuf::from(
            std::env::var("COMPANY_FIXTURE_DB")
                .expect("set COMPANY_FIXTURE_DB to a copy of the database"),
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

        // The RAW key as stored, not the canonicalized one — otherwise the table hides the
        // very pairs this is meant to re-propose.
        let keys: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT company_key FROM internship_postings ORDER BY 1")
                .fetch_all(&pool)
                .await
                .expect("read company keys");

        let table = AliasTable::vendored();
        let refused: BTreeSet<&str> = table.refused().iter().map(|r| r.candidate.as_str()).collect();
        let candidates = propose(&keys);

        let mut fresh = 0;
        println!("\n=== 12d: {} company keys, {} candidates ===", keys.len(), candidates.len());
        for candidate in &candidates {
            let verdict = if table.aliases().contains_key(&candidate.longer) {
                "merged"
            } else if refused.contains(candidate.longer.as_str()) {
                "refused"
            } else {
                fresh += 1;
                "NEW — needs review"
            };
            println!(
                "  {verdict:<18} {:<34} + {:<28} ({})",
                candidate.shorter, candidate.added, candidate.longer
            );
        }
        println!("\n  {fresh} candidate(s) the committed file has no opinion on.");

        let Ok(out) = std::env::var("COMPANY_MIGRATION_OUT") else {
            return;
        };

        // ---- generate migration 0029 from the same table the runtime uses ----

        #[derive(sqlx::FromRow)]
        struct Row {
            id: String,
            dedup_key: String,
            company_key: String,
            sightings: i64,
            apps: i64,
            events: i64,
        }

        let rows = sqlx::query_as::<_, Row>(
            "SELECT p.id, p.dedup_key, p.company_key,
                    (SELECT COUNT(*) FROM posting_sightings s WHERE s.posting_id = p.id) AS sightings,
                    (SELECT COUNT(*) FROM internship_applications a WHERE a.posting_id = p.id) AS apps,
                    (SELECT COUNT(*) FROM hunt_events h WHERE h.subject_id = p.id) AS events
               FROM internship_postings p ORDER BY p.id",
        )
        .fetch_all(&pool)
        .await
        .expect("read postings");

        // New keys, from the committed table — never from a second parser.
        let mut plan: Vec<(&Row, String, String)> = Vec::new();
        for row in &rows {
            let canonical = table.canonical(&row.company_key);
            if canonical == row.company_key {
                continue;
            }
            // `company_key` cannot contain '|' (it is space-joined tokens), so the company
            // component of a fallback key ends at the first one.
            let new_dedup = match row.dedup_key.strip_prefix("co:") {
                Some(body) => match body.split_once('|') {
                    Some((company, rest)) if company == row.company_key => {
                        format!("co:{canonical}|{rest}")
                    }
                    _ => row.dedup_key.clone(),
                },
                None => row.dedup_key.clone(),
            };
            plan.push((row, canonical.to_string(), new_dedup));
        }

        // Groups that end up sharing a key: one survives, the rest are merged into it.
        let mut by_new_key: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
        for (row, _, new_dedup) in &plan {
            by_new_key.entry(new_dedup.clone()).or_default().push(row);
        }
        // A key already held by a row this migration does not touch would collide silently.
        let touched: BTreeSet<&str> = plan.iter().map(|(r, _, _)| r.id.as_str()).collect();
        for new_key in by_new_key.keys() {
            for row in &rows {
                assert!(
                    row.dedup_key != *new_key || touched.contains(row.id.as_str()),
                    "new key {new_key} is already held by untouched posting {}",
                    row.id
                );
            }
        }

        let mut sql = String::new();
        let mut merged = 0;
        for (new_key, group) in &by_new_key {
            if group.len() < 2 {
                continue;
            }
            // Survive the row with the most sightings: the most evidence, and the fewest rows
            // to move. Ties break on id so the file is deterministic.
            let mut ordered = group.clone();
            ordered.sort_by(|a, b| b.sightings.cmp(&a.sightings).then(a.id.cmp(&b.id)));
            let (survivor, losers) = ordered.split_first().expect("group is non-empty");
            sql.push_str(&format!("-- merge into {} ({new_key})\n", survivor.id));
            for loser in losers {
                merged += 1;
                let (win, lose) = (&survivor.id, &loser.id);
                sql.push_str(&format!(
                    "\n-- {lose} -> {win}\n\
                     UPDATE internship_applications SET posting_id = '{win}'\n\
                     \u{20} WHERE posting_id = '{lose}'\n\
                     \u{20}   AND NOT EXISTS (SELECT 1 FROM internship_applications other\n\
                     \u{20}                    WHERE other.user_id = internship_applications.user_id\n\
                     \u{20}                      AND other.posting_id = '{win}');\n\
                     UPDATE posting_sightings SET posting_id = '{win}' WHERE posting_id = '{lose}';\n\
                     DELETE FROM hunt_events WHERE subject_id = '{lose}'\n\
                     \u{20}   AND EXISTS (SELECT 1 FROM hunt_events other\n\
                     \u{20}                WHERE other.kind = hunt_events.kind\n\
                     \u{20}                  AND other.subject_id = '{win}');\n\
                     UPDATE hunt_events SET subject_id = '{win}' WHERE subject_id = '{lose}';\n\
                     DELETE FROM internship_postings WHERE id = '{lose}'\n\
                     \u{20}   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = '{lose}')\n\
                     \u{20}   AND NOT EXISTS (SELECT 1 FROM posting_sightings s WHERE s.posting_id = '{lose}')\n\
                     \u{20}   AND NOT EXISTS (SELECT 1 FROM hunt_events h WHERE h.subject_id = '{lose}');\n"
                ));
            }
        }

        sql.push_str("\n-- re-key the survivors and every other renamed posting\n");
        for (row, canonical, new_dedup) in &plan {
            sql.push_str(&format!(
                "UPDATE internship_postings SET company_key = '{canonical}', dedup_key = '{new_dedup}'\n WHERE id = '{}';\n",
                row.id
            ));
        }

        std::fs::write(&out, &sql).expect("write the migration body");
        println!(
            "\n  {} postings re-keyed, {} merged away; wrote {out}",
            plan.len(),
            merged
        );
        for (row, _, _) in &plan {
            assert!(
                !(row.apps > 0 && row.events > 0),
                "posting {} carries both an application and an alert; review by hand",
                row.id
            );
        }
    }

    #[test]
    fn propose_does_not_treat_a_shared_word_prefix_as_a_token_prefix() {
        // `andurilx` starts with the letters of `anduril` and is a different company. Matching
        // on characters rather than tokens is the classic version of this bug — the same one
        // `guess_company` had, where PPL matched the inside of "apply".
        let keys: Vec<String> = ["anduril", "andurilx industries"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(propose(&keys).is_empty());
    }
}
