//! `boards discover`: grow the board directory from URLs the collector has already seen.
//!
//! # Why this exists
//!
//! `data/internships/board-slugs.json` is the list of ATS boards the collector polls. It is a
//! committed snapshot, derived once — on 2026-09-05 — from Simplify's `listings.json` by
//! `simplify::extract_board_slugs`, and **never regenerated since**. `BoardDirectory::merge` was
//! written for exactly this ("merge freshly discovered slugs in") and so was the harvester; both
//! were only ever called from tests. At runtime the collector polls `BoardDirectory::vendored()`
//! and nothing else, so every employer Simplify started listing after 2026-09-05 is invisible to
//! the ATS sources.
//!
//! Measured on 2026-09-30 from the URLs already in `posting_sightings`: **118 pollable boards**
//! carrying accepted internship postings that the directory does not contain.
//!
//! # And what it does not do, said plainly
//!
//! The question that prompted this was "find companies without going through Simplify".
//! Harvesting every source the collector already reads answers it: **3 boards** are reachable
//! from vanshb03 and the ATS sources alone. The other 115 are Simplify's own, found late. The
//! ceiling in `docs/INTERNSHIP_SCRAPING.md` § G stands — new employers at scale need an
//! independent list that does not exist in this corpus — but it was the lower of two ceilings,
//! and the higher one was a stale file.
//!
//! # Why a command that writes the file, and not a merge at runtime
//!
//! The directory is reviewed operational data, and `data/internships/README.md` treats it that
//! way: retiring a board is a hand edit recorded in `retired-slugs.json`, and "stays a human's
//! decision". A runtime merge would change what gets polled on every run with no diff anybody
//! reads. This produces one, dry by default, and `--apply` writes it for review in `git diff`.
//!
//! # Retired boards stay retired
//!
//! The README's own warning: "`simplify::extract_board_slugs` rebuilds it … and nothing in that
//! pipeline knows a slug was retired, so without the record the next harvest silently
//! resurrects every one of them." It would have, here: **4** of the harvested boards are
//! retired, dead ones. They are excluded by `retired-slugs.json`, case-insensitively, since
//! Ashby answers to `AtomicSemi` and `atomicsemi` alike.
//!
//! No network requests. Everything read here is already in the database or on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use sqlx::SqlitePool;

use super::sources::simplify::board_of;

const DIRECTORY: &str = "data/internships/board-slugs.json";
const RETIRED: &str = "data/internships/retired-slugs.json";

/// The ATSes a source adapter exists for and is permitted to poll. The others are recorded
/// because harvesting them is free — see the README — but adding to them changes no run.
const POLLED: &[&str] = &["ashby", "greenhouse", "lever", "workable"];

/// A board the harvest found that the directory does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub ats: String,
    pub slug: String,
    /// Every source whose URLs named it — so "reachable without Simplify" is checkable.
    pub sources: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub new: Vec<Found>,
    /// Harvested, but recorded as retired. Excluded, and listed so the exclusion is visible.
    pub still_retired: Vec<(String, String)>,
    pub already_known: usize,
}

type Directory = BTreeMap<String, Vec<String>>;

fn key(ats: &str, slug: &str) -> (String, String) {
    (ats.to_string(), slug.to_lowercase())
}

/// The pure part: given what was seen and what is on file, what is new.
///
/// `seen` is `(source, url)`. Case-insensitive throughout, keeping whichever spelling the
/// directory already has, and otherwise the first one seen.
pub fn discover(
    seen: &[(String, String)],
    directory: &Directory,
    retired: &BTreeSet<(String, String)>,
) -> Discovery {
    let known: BTreeSet<(String, String)> = directory
        .iter()
        .flat_map(|(ats, slugs)| slugs.iter().map(move |slug| key(ats, slug)))
        .collect();

    let mut found: BTreeMap<(String, String), Found> = BTreeMap::new();
    let mut discovery = Discovery::default();
    let mut counted_known: BTreeSet<(String, String)> = BTreeSet::new();
    let mut counted_retired: BTreeSet<(String, String)> = BTreeSet::new();

    for (source, url) in seen {
        let Some((ats, slug)) = board_of(url) else {
            continue;
        };
        let k = key(ats, &slug);
        if known.contains(&k) {
            if counted_known.insert(k) {
                discovery.already_known += 1;
            }
            continue;
        }
        if retired.contains(&k) {
            if counted_retired.insert(k) {
                discovery.still_retired.push((ats.to_string(), slug));
            }
            continue;
        }
        found
            .entry(k)
            .or_insert_with(|| Found {
                ats: ats.to_string(),
                slug: slug.clone(),
                sources: BTreeSet::new(),
            })
            .sources
            .insert(source.clone());
    }

    discovery.new = found.into_values().collect();
    discovery
}

/// Add the new boards, keeping each list in the byte order the file already uses.
pub fn merged(directory: &Directory, new: &[Found]) -> Directory {
    let mut out = directory.clone();
    for board in new {
        let slugs = out.entry(board.ats.clone()).or_default();
        if !slugs.iter().any(|s| s.eq_ignore_ascii_case(&board.slug)) {
            slugs.push(board.slug.clone());
        }
    }
    for slugs in out.values_mut() {
        slugs.sort();
    }
    out
}

/// Serialise in the file's own format — one-space indent, no trailing newline — so a regenerated
/// directory diffs as the lines that changed and nothing else.
pub fn render(directory: &Directory) -> Result<String> {
    let mut buffer = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    serde::Serialize::serialize(directory, &mut serializer)?;
    Ok(String::from_utf8(buffer)?)
}

fn read_retired(path: &Path) -> Result<BTreeSet<(String, String)>> {
    #[derive(serde::Deserialize)]
    struct Entry {
        source: String,
        slug: String,
    }
    #[derive(serde::Deserialize)]
    struct File {
        retired: Vec<Entry>,
    }
    let file: File = serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    Ok(file.retired.iter().map(|e| key(&e.source, &e.slug)).collect())
}

const USAGE: &str = "\
usage:
  boards discover [--apply]

Harvests ATS board slugs from every URL already in the database — every source's sightings,
and your tracked applications — and reports the boards `data/internships/board-slugs.json`
does not contain. Retired boards are never added back. No network requests.

  --apply   write the additions into data/internships/board-slugs.json, for review in
            `git diff`. The file is compiled into the binary: rebuild and restart afterwards.
";

pub async fn main(pool: &SqlitePool, args: &[String]) -> Result<()> {
    let apply = match args {
        [] => false,
        [flag] if flag == "--apply" => true,
        _ => {
            print!("{USAGE}");
            bail!("unrecognized arguments to `boards discover`");
        }
    };

    let mut seen: Vec<(String, String)> =
        sqlx::query_as("SELECT DISTINCT source, url FROM posting_sightings WHERE url IS NOT NULL")
            .fetch_all(pool)
            .await?;
    let applications: Vec<String> =
        sqlx::query_scalar("SELECT url FROM internship_applications WHERE url IS NOT NULL")
            .fetch_all(pool)
            .await?;
    seen.extend(applications.into_iter().map(|url| ("tracked applications".to_string(), url)));

    // Read from disk rather than the compiled-in copy: this is about to rewrite the file, and a
    // binary built before someone's hand edit would merge into what the file used to say.
    let directory: Directory = serde_json::from_str(
        &std::fs::read_to_string(DIRECTORY).with_context(|| format!("reading {DIRECTORY}"))?,
    )?;
    let retired = read_retired(Path::new(RETIRED))?;

    let discovery = discover(&seen, &directory, &retired);

    let polled: Vec<&Found> = discovery.new.iter().filter(|f| POLLED.contains(&f.ats.as_str())).collect();
    let independent: Vec<&Found> =
        discovery.new.iter().filter(|f| f.sources.iter().any(|s| s != "simplify")).collect();

    println!(
        "boards discover: {} new ({} on an ATS that is polled), {} already on file",
        discovery.new.len(),
        polled.len(),
        discovery.already_known
    );
    let mut per_ats: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &discovery.new {
        *per_ats.entry(f.ats.as_str()).or_default() += 1;
    }
    for (ats, n) in &per_ats {
        let note = if POLLED.contains(ats) { "" } else { "  (recorded, not polled)" };
        println!("  {ats:<16} +{n}{note}");
    }
    println!(
        "\nreachable without Simplify: {}{}",
        independent.len(),
        if independent.is_empty() { String::new() } else {
            format!(
                "  — {}",
                independent.iter().map(|f| format!("{}/{}", f.ats, f.slug)).collect::<Vec<_>>().join(", ")
            )
        }
    );
    if !discovery.still_retired.is_empty() {
        println!(
            "\n{} harvested board(s) are retired and were NOT added back: {}",
            discovery.still_retired.len(),
            discovery
                .still_retired
                .iter()
                .map(|(a, s)| format!("{a}/{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if discovery.new.is_empty() {
        return Ok(());
    }
    if !apply {
        println!("\n  re-run with --apply to write them into {DIRECTORY}.");
        return Ok(());
    }

    std::fs::write(DIRECTORY, render(&merged(&directory, &discovery.new))?)
        .with_context(|| format!("writing {DIRECTORY}"))?;
    println!(
        "\nwrote {DIRECTORY}. It is compiled in: rebuild and restart the backend, and review the \
         additions with `git diff` before committing."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(pairs: &[(&str, &[&str])]) -> Directory {
        pairs
            .iter()
            .map(|(ats, slugs)| (ats.to_string(), slugs.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn seen(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(s, u)| (s.to_string(), u.to_string())).collect()
    }

    /// The README's warning, made executable: a harvest must not resurrect a board somebody
    /// retired, and Ashby slugs differ only in case often enough that the check has to ignore it.
    #[test]
    fn a_retired_board_is_never_added_back_whatever_its_case() {
        let directory = dir(&[("ashby", &[])]);
        let retired = BTreeSet::from([key("ashby", "atomicsemi")]);
        let found = discover(
            &seen(&[("vanshb03", "https://jobs.ashbyhq.com/AtomicSemi/123")]),
            &directory,
            &retired,
        );
        assert!(found.new.is_empty(), "{:?}", found.new);
        assert_eq!(found.still_retired, vec![("ashby".to_string(), "AtomicSemi".to_string())]);
    }

    /// A board already on file under another spelling is on file.
    #[test]
    fn a_known_board_in_another_case_is_not_a_new_one() {
        let directory = dir(&[("lever", &["AIFund"])]);
        let found = discover(
            &seen(&[("simplify", "https://jobs.lever.co/aifund/abc")]),
            &directory,
            &BTreeSet::new(),
        );
        assert!(found.new.is_empty());
        assert_eq!(found.already_known, 1);
    }

    /// Provenance is kept, which is what lets the report say how much is Simplify-independent.
    #[test]
    fn a_new_board_remembers_every_source_that_named_it() {
        let found = discover(
            &seen(&[
                ("simplify", "https://job-boards.greenhouse.io/acme/jobs/1"),
                ("vanshb03", "https://boards.greenhouse.io/acme/jobs/2"),
            ]),
            &dir(&[("greenhouse", &[])]),
            &BTreeSet::new(),
        );
        assert_eq!(found.new.len(), 1, "two hosts, one board");
        assert_eq!(
            found.new[0].sources,
            BTreeSet::from(["simplify".to_string(), "vanshb03".to_string()])
        );
    }

    /// Additive only, and sorted the way the file already is.
    #[test]
    fn merging_adds_in_byte_order_and_removes_nothing() {
        let directory = dir(&[("greenhouse", &["Zeta", "alpha"])]);
        let new = vec![Found {
            ats: "greenhouse".to_string(),
            slug: "Beta".to_string(),
            sources: BTreeSet::new(),
        }];
        assert_eq!(
            merged(&directory, &new)["greenhouse"],
            vec!["Beta".to_string(), "Zeta".to_string(), "alpha".to_string()]
        );
    }

    /// The committed directory round-trips through `render` byte for byte, so `--apply` diffs as
    /// exactly the lines it added. A formatter that reflowed the file would bury 118 additions
    /// in 2,000 lines of whitespace.
    #[test]
    fn rendering_the_committed_directory_reproduces_it_exactly() {
        let committed = include_str!("../../data/internships/board-slugs.json");
        let parsed: Directory = serde_json::from_str(committed).expect("parses");
        assert_eq!(render(&parsed).expect("renders"), committed);
    }
}
