
use serde::Serialize;
use strsim::normalized_damerau_levenshtein;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionSource {
    /// Item present in fridge
    Fridge,
    /// Item present in FoodKeeper
    Foodkeeper,
}

/// A struct documenting the features of an item
#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub name_lower: String,
    pub source: SuggestionSource,
    /// Common aliases for the item
    pub aliases: Vec<String>,
    pub foodkeeper_product_id: Option<i64>,
}

/// A struct with fields for indicating how relevant an item is based on score
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub name: String,
    pub source: SuggestionSource,
    pub foodkeeper_product_id: Option<i64>,
    /// Assigned score
    pub score: f32,
}

/// A banded recommendation system. Exact matches are the highest band. Prefixes
/// are the next, then substrings, then fuzzy scoring. The bands do not intersect
/// so a strong fuzzy score will never outrank a substring score.
const BAND_EXACT: f32 = 1.00;
const BAND_PREFIX: f32 = 0.80;
const BAND_SUBSTRING: f32 = 0.60;
const BAND_FUZZY: f32 = 0.30;

/// Aliases are a weaker indication of intent, so give a penalty constant to alias matching
const ALIAS_CONST: f32 = 0.1;

/// A constraint on the bounds of a band so that bands do not overlap
const BAND_WIDTH: f32 = 0.09;

/// Exclude suggestions with a score below this bound:
const SCORE_FLOOR: f32 = BAND_FUZZY;

/// Normalizes a string trimming excess whitespace and sending all characters to lowercase
fn normalize(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Assigns a score to all of the possible candidates input candidates. Then sorts
/// by the score and truncates the output vector to the limit.
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

    // First sorts by score, but then as a tie-break scores fridge before catalog items
    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.source.cmp(&b.source))
    });
    scored.truncate(limit);
    scored
}

/// The actual score assignment function. Alias matches are given a flat penalty.
/// Prefix and substring matches are given a scaling factor based on how much of
/// the actual query the prefix or substring occupies. Prefixes are matched against
/// all words in a multi-word phrase. Prefixes and substrings are compared against
/// the canonical name and its aliases. Fuzzy scoring is conducted using a
/// normalized_damerau_levenshtein function and words that score too low are
/// dropped due to being too low quality.
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
        let results = suggest_item_names("tom", &candidates(), 5);
        assert!(
            rank_of(&results, "tomatoes").is_some(),
            "expected 'tom' to suggest 'Tomatoes', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn prefix_ranks_by_coverage() {
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
        let results = suggest_item_names("clery", &candidates(), 5);
        assert!(
            rank_of(&results, "celery").is_some(),
            "expected 'clery' to suggest 'Celery', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn plural_matches_singular_name() {
        let results = suggest_item_names("potatoes", &candidates(), 5);
        assert!(
            rank_of(&results, "potato").is_some(),
            "expected 'potatoes' to suggest 'Potato', got {:?}",
            names(&results)
        );
    }

    #[test]
    fn keyword_alias_matches() {
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
        assert!(suggest_item_names("", &candidates(), 5).is_empty());
        assert!(suggest_item_names("   ", &candidates(), 5).is_empty());
    }
}
