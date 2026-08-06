//! Item name suggestion — flagged as a learning area, see CLAUDE.md.
//!
//! Goal: given what the user has typed so far, return the best few known item names to
//! show in the add-item dropdown, ranked best-first.
//!
//! This is a *ranking* problem, not a decide-for-the-user problem. Nothing here is
//! committed without the user picking it, so returning a mediocre 5th suggestion costs
//! nothing — tune for recall, not precision.
//!
//! TODO(you): replace the body of `suggest_item_names`. The plan is a banded tier stack,
//! where each tier owns a disjoint score range so ranking stays explainable:
//!
//!   1.0        exact match (normalized)
//!   0.80-0.95  prefix match — whole string OR any token ("oil" -> "olive oil"),
//!              ranked within the band by coverage (query.len / candidate.len)
//!   0.60-0.80  substring / containment ("mato" -> "tomato")
//!   0.30-0.60  normalized Damerau-Levenshtein ("tomatoe", "tomtao")
//!
//! Run the whole stack against `Candidate::name` *and* every entry in `Candidate::aliases`,
//! taking the best score. Then run it a second time over stemmed/singularized forms at a
//! small discount (~0.95x) to pick up plurals without letting stemming contaminate the
//! prefix scores.
//!
//! Suggested crates (neither is a dependency yet — add what you use):
//!   strsim = "0.11.1"         normalized_damerau_levenshtein for the typo tier
//!   rust-stemmers = "1.2.0"   Snowball English stemmer for the plural pass
//!
//! The tests below describe the behavior this needs. They fail against the current
//! placeholder, which only does exact matching.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// Ranked best-first, at most `limit` entries, empty when nothing clears the floor.
///
/// `query` is the raw text typed so far. An empty query is *not* this function's job —
/// the route serves recent items in that case, so returning an empty Vec is correct.
pub fn suggest_item_names(query: &str, candidates: &[Candidate], limit: usize) -> Vec<Suggestion> {
    // Placeholder: exact case-insensitive match on name or alias, nothing else. The
    // dropdown will look nearly dead until you implement the tiers above — a query only
    // matches once it's typed out in full. That's the gap the tests below describe.
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    candidates
        .iter()
        .filter(|candidate| {
            candidate.name.to_lowercase() == normalized
                || candidate.aliases.iter().any(|alias| *alias == normalized)
        })
        .take(limit)
        .map(|candidate| Suggestion {
            name: candidate.name.clone(),
            source: candidate.source,
            foodkeeper_product_id: candidate.foodkeeper_product_id,
            score: 1.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fridge(name: &str) -> Candidate {
        Candidate {
            name: name.to_string(),
            source: SuggestionSource::Fridge,
            aliases: Vec::new(),
            foodkeeper_product_id: None,
        }
    }

    fn foodkeeper(name: &str, id: i64, aliases: &[&str]) -> Candidate {
        Candidate {
            name: name.to_string(),
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
