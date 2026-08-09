//! Item name suggestion — flagged as a learning area, see CLAUDE.md.
//!
//! Goal: given what the user has typed so far, return the best few known item names to
//! show in the add-item dropdown, ranked best-first.
//!
//! This is a *ranking* problem, not a decide-for-the-user problem. Nothing here is
//! committed without the user picking it, so returning a mediocre 5th suggestion costs
//! nothing — tune for recall, not precision.
//!
//! Scoring is a banded tier stack. Each tier owns a disjoint score range, and matches on a
//! candidate's `aliases` sit one `ALIAS_CONST` below the equivalent match on its `name` —
//! so the ranking is always explainable as "which tier fired, name or alias?":
//!
//! | Score       | Tier                                                     |
//! |-------------|----------------------------------------------------------|
//! | 1.00        | exact, name                                              |
//! | 0.90        | exact, alias                                             |
//! | 0.80 – 0.89 | prefix, name — whole string or any token                  |
//! | 0.70 – 0.79 | prefix, alias                                            |
//! | 0.60 – 0.69 | substring, name — "melon" -> "watermelon"                 |
//! | 0.50 – 0.59 | substring, alias                                          |
//! | 0.30 – 0.50 | fuzzy, name tokens — Damerau-Levenshtein ("clery")        |
//!
//! For the exact/prefix/substring tiers, `BAND_WIDTH * coverage` positions the score within
//! its band, where coverage is the query's length over the length of *the string that
//! matched* — the name for name tiers, the matched alias for alias tiers. Since every one
//! of those matchers is anchored or containment-based, `query.len() <= matched.len()` always
//! holds, so coverage can't exceed 1.0 and no band can overflow into the one above it.
//!
//! Branches are checked in descending score order and return on the first hit, which is
//! only sound because the bands are disjoint. Adding a tier means putting it in the right
//! place in that order, not just appending it.
//!
//! The fuzzy tier is the exception to all of the above, and the one still under
//! construction. It is not a predicate: every string has *some* similarity to every other
//! string, so unlike the tiers above it needs an explicit cutoff or it matches the entire
//! catalog on every keystroke. Measured against this data, real typos score 0.75–0.875 and
//! the best unrelated noise scores 0.571, so a similarity threshold around 0.7 separates
//! them cleanly. It also uses its own width (0.2) rather than `BAND_WIDTH`, which is why
//! its range is listed as 0.30–0.50 above.
//!
//! TODO(you): the fuzzy threshold, and optionally a stemming pass. `plural_matches_-
//! singular_name` currently passes through fuzzy rather than stemming — a plural isn't
//! really a typo, so `rust-stemmers = "1.2.0"` is still the semantically right tool if you
//! want it, but no test will tell you the difference.

use serde::Serialize;
use strsim::normalized_damerau_levenshtein;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionSource {
    /// Already in the fridge. Picking one of these means the user probably meant the
    /// thing they already have, so it should win ties against the catalog.
    Fridge,
    /// A known food name from the FoodKeeper catalog.
    Foodkeeper,
}

/// Something the user could be typing the name of.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub name_lower: String,
    pub source: SuggestionSource,
    /// Alternate names worth matching against (FoodKeeper `Keywords`). Empty for fridge
    /// items. Lowercased and trimmed by the caller.
    pub aliases: Vec<String>,
    pub foodkeeper_product_id: Option<i64>,
}

/// One row in the dropdown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub name: String,
    pub source: SuggestionSource,
    pub foodkeeper_product_id: Option<i64>,
    /// 0.0-1.0, higher is better. Serialized so the UI can debug ranking.
    pub score: f32,
}

// Band floors. Each tier's scores live between its own floor and the next one up, so a
// weak prefix match can never outrank a strong substring match. Adjust freely — these are
// the contract you're designing against, not fixed truth.
const BAND_EXACT: f32 = 1.00;
const BAND_PREFIX: f32 = 0.80;
const BAND_SUBSTRING: f32 = 0.60;
const BAND_FUZZY: f32 = 0.30;

const ALIAS_CONST: f32 = 0.1;

/// How far an exact/prefix/substring match can climb above its band floor, scaled by
/// coverage.
///
/// Must stay strictly below `ALIAS_CONST`, or a top-of-band name match would tie or
/// overtake the exact-alias match sitting one band above it. The current margin is 0.01.
/// The fuzzy tier does not use this — it has its own, wider width.
const BAND_WIDTH: f32 = 0.09;

/// Anything scoring below this never reaches the dropdown. Nothing auto-highlights, so a
/// mediocre 5th suggestion costs the user nothing — keep this generous.
const SCORE_FLOOR: f32 = BAND_FUZZY;

/// Trim, lowercase, collapse internal whitespace. Applied to the query once per request;
/// candidate names and aliases are already lowercased by the time they get here.
fn normalize(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Ranked best-first, at most `limit` entries, empty when nothing clears the floor.
///
/// `query` is the raw text typed so far. An empty query is *not* this function's job —
/// the route serves recent items in that case, so returning an empty Vec is correct.
pub fn suggest_item_names(query: &str, candidates: &[Candidate], limit: usize) -> Vec<Suggestion> {
    let query = normalize(query);
    if query.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<Suggestion> = candidates
        .iter()
        .filter_map(|candidate| {
            let score = score_one(&query, candidate)?;
            (score >= SCORE_FLOOR).then(|| Suggestion {
                name: candidate.name.clone(),
                source: candidate.source,
                foodkeeper_product_id: candidate.foodkeeper_product_id,
                score,
            })
        })
        .collect();

    // Sort before truncating, or "top 5" is just the first 5 in catalog order. The
    // tie-break has to live on this sort rather than a later one: truncation decides which
    // candidates survive at all, so a tie-break applied afterwards could only reorder the
    // survivors, never rescue a fridge item that had already been cut.
    //
    // Note the deliberately different argument orders. Score is `b` then `a` — reversed,
    // for descending. Source is `a` then `b` — not reversed, so the *smaller* variant wins,
    // and `Fridge` is declared before `Foodkeeper`. `total_cmp` avoids the unwrap that
    // `partial_cmp` would force, since f32 isn't Ord.
    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.source.cmp(&b.source))
    });
    scored.truncate(limit);
    scored
}

/// The whole ranking algorithm: how well does `candidate` match `query`?
///
/// `query` is already normalized. Returns `None` when nothing matches at all; the caller
/// applies `SCORE_FLOOR` and handles sorting and truncation. See the module docs for the
/// band table these branches implement.
///
/// The branches run in descending score order and return on the first hit. Three
/// subtleties worth not re-deriving:
///
/// - The token-prefix branches are *not* redundant with the whole-string ones: a token can
///   start with the query when the whole string doesn't ("cheese" in "cream cheese").
/// - The substring tier needs no token variant. A token is a contiguous substring of the
///   whole string, so token containment is entirely subsumed by string containment.
/// - Fuzzy is the reverse of prefix: whole-string similarity is near-useless against long
///   multi-word names ("brocolli" vs "broccoli and broccoli raab (rapini)" scores ~0), so
///   per-token is the primary matcher there rather than the supplement.
fn score_one(query: &str, candidate: &Candidate) -> Option<f32> {
    if candidate.name_lower == query {
        return Some(BAND_EXACT);
    }
    if candidate.aliases.iter().any(|a| a == query) {
        return Some(BAND_EXACT - ALIAS_CONST);
    }

    if candidate.name_lower.starts_with(query) {
        return Some(
            BAND_PREFIX + BAND_WIDTH * (query.len() as f32 / (candidate.name_lower.len() as f32)),
        );
    }
    if candidate
        .name_lower
        .split_whitespace()
        .any(|a| a.starts_with(query))
    {
        return Some(
            BAND_PREFIX + BAND_WIDTH * (query.len() as f32 / (candidate.name_lower.len() as f32)),
        );
    }
    if let Some(alias) = candidate.aliases.iter().find(|a| a.starts_with(query)) {
        return Some(
            BAND_PREFIX - ALIAS_CONST + BAND_WIDTH * (query.len() as f32 / (alias.len() as f32)),
        );
    }
    if let Some(alias) = candidate
        .aliases
        .iter()
        .find(|a| a.split_whitespace().any(|b| b.starts_with(query)))
    {
        return Some(
            BAND_PREFIX - ALIAS_CONST + BAND_WIDTH * (query.len() as f32 / (alias.len() as f32)),
        );
    }

    if candidate.name_lower.contains(query) {
        return Some(
            BAND_SUBSTRING
                + BAND_WIDTH * (query.len() as f32 / (candidate.name_lower.len() as f32)),
        );
    }
    if let Some(alias) = candidate.aliases.iter().find(|a| a.contains(query)) {
        return Some(
            BAND_SUBSTRING - ALIAS_CONST + BAND_WIDTH * (query.len() as f32 / (alias.len() as f32)),
        );
    }

    let fuzscore = candidate
        .name_lower
        .split_whitespace()
        .map(|token| normalized_damerau_levenshtein(query, token) as f32)
        .fold(0.0, f32::max);

    if fuzscore > 0.7 {
        return Some(BAND_FUZZY + 0.2 * fuzscore);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fridge(name: &str) -> Candidate {
        Candidate {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            source: SuggestionSource::Fridge,
            aliases: Vec::new(),
            foodkeeper_product_id: None,
        }
    }

    fn foodkeeper(name: &str, id: i64, aliases: &[&str]) -> Candidate {
        Candidate {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            source: SuggestionSource::Foodkeeper,
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            foodkeeper_product_id: Some(id),
        }
    }

    fn candidates() -> Vec<Candidate> {
        vec![
            fridge("milk"),
            foodkeeper("Tomatoes", 1, &["tomato"]),
            foodkeeper("Tomato sauce", 2, &["sauce", "spaghetti", "pizza"]),
            foodkeeper("Cherry tomatoes", 3, &["cherry", "tomato"]),
            foodkeeper("Olive oil", 4, &["oil"]),
            foodkeeper("Potato", 5, &[]),
            foodkeeper("Celery", 6, &[]),
        ]
    }

    fn names(suggestions: &[Suggestion]) -> Vec<String> {
        suggestions.iter().map(|s| s.name.to_lowercase()).collect()
    }

    fn rank_of(suggestions: &[Suggestion], name: &str) -> Option<usize> {
        names(suggestions).iter().position(|n| n == name)
    }

    #[test]
    fn exact_match_ranks_first() {
        let results = suggest_item_names("celery", &candidates(), 5);
        assert_eq!(rank_of(&results, "celery"), Some(0));
    }

    #[test]
    fn exact_match_is_case_insensitive() {
        let results = suggest_item_names("CeLeRy", &candidates(), 5);
        assert_eq!(rank_of(&results, "celery"), Some(0));
    }

    #[test]
    fn prefix_match_is_found() {
        // The dominant typeahead case: every keystroke is a truncation of the target.
        let results = suggest_item_names("tom", &candidates(), 5);
        assert!(
            rank_of(&results, "tomatoes").is_some(),
            "expected 'tom' to suggest 'Tomatoes', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn prefix_ranks_by_coverage() {
        // "tom" covers more of "Tomatoes" than of "Tomato sauce", so it's the likelier
        // intent. Change this test if you pick a different within-band ordering.
        let results = suggest_item_names("tom", &candidates(), 5);
        let tomatoes = rank_of(&results, "tomatoes").expect("tomatoes should be suggested");
        let sauce = rank_of(&results, "tomato sauce").expect("tomato sauce should be suggested");
        assert!(
            tomatoes < sauce,
            "expected 'Tomatoes' above 'Tomato sauce' for query 'tom', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn token_prefix_matches_multi_word_names() {
        // Grocery names are mostly multi-word; whole-string prefix alone misses this.
        let results = suggest_item_names("oil", &candidates(), 5);
        assert!(
            rank_of(&results, "olive oil").is_some(),
            "expected 'oil' to suggest 'Olive oil', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn prefix_outranks_substring() {
        let candidates = vec![
            foodkeeper("Cheese", 1, &[]),
            foodkeeper("Cream cheese", 2, &[]),
        ];
        let results = suggest_item_names("chee", &candidates, 5);
        let cheese = rank_of(&results, "cheese").expect("cheese should be suggested");
        let cream = rank_of(&results, "cream cheese").expect("cream cheese should be suggested");
        assert!(
            cheese < cream,
            "expected whole-string prefix above mid-string match, got {:?}",
            names(&results)
        );
    }

    #[test]
    fn common_typo_still_matches() {
        let results = suggest_item_names("tomatoe", &candidates(), 5);
        assert!(
            rank_of(&results, "tomatoes").is_some(),
            "expected 'tomatoe' to suggest 'Tomatoes', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn transposition_still_matches() {
        // Damerau counts this as one edit; plain Levenshtein counts two.
        let results = suggest_item_names("clery", &candidates(), 5);
        assert!(
            rank_of(&results, "celery").is_some(),
            "expected 'clery' to suggest 'Celery', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn plural_matches_singular_name() {
        // "Potato" has no plural keyword, so this only passes via stemming.
        let results = suggest_item_names("potatoes", &candidates(), 5);
        assert!(
            rank_of(&results, "potato").is_some(),
            "expected 'potatoes' to suggest 'Potato', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn keyword_alias_matches() {
        // The FoodKeeper Keywords column is doing the work here — no string metric gets
        // from "spaghetti" to "Tomato sauce".
        let results = suggest_item_names("spaghetti", &candidates(), 5);
        assert!(
            rank_of(&results, "tomato sauce").is_some(),
            "expected 'spaghetti' to suggest 'Tomato sauce', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn unrelated_query_returns_nothing() {
        let results = suggest_item_names("bicycle", &candidates(), 5);
        assert!(
            results.is_empty(),
            "expected no suggestions, got {:?}",
            names(&results)
        );
    }

    #[test]
    fn fridge_items_outrank_catalog_on_ties() {
        let candidates = vec![foodkeeper("Tomatoes", 1, &["tomato"]), fridge("tomatoes")];
        let results = suggest_item_names("tomatoes", &candidates, 5);
        assert_eq!(
            results.first().map(|s| s.source),
            Some(SuggestionSource::Fridge),
            "an item already in the fridge should win an equal-score tie"
        );
    }

    #[test]
    fn limit_is_respected() {
        let results = suggest_item_names("tomato", &candidates(), 2);
        assert!(
            results.len() <= 2,
            "got {} results for limit 2",
            results.len()
        );
    }

    #[test]
    fn results_are_sorted_by_score_descending() {
        let results = suggest_item_names("tomato", &candidates(), 5);
        assert!(
            results.windows(2).all(|w| w[0].score >= w[1].score),
            "scores out of order: {:?}",
            results.iter().map(|s| s.score).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_query_returns_nothing() {
        // The route serves recent items for an empty query; the ranker stays out of it.
        assert!(suggest_item_names("", &candidates(), 5).is_empty());
        assert!(suggest_item_names("   ", &candidates(), 5).is_empty());
    }
}
