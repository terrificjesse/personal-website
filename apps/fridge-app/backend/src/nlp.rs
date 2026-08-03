//! Item name resolution — flagged as a learning area, see CLAUDE.md.
//!
//! Goal: when a user types a new item name, decide whether it refers to an item that
//! already exists in the fridge (typos, pluralization, casing) or is genuinely new.
//!
//! TODO(you): replace the body of `resolve_item_name` with real matching logic.
//! Ideas to research: Levenshtein/Damerau-Levenshtein edit distance, naive plural
//! stemming (strip trailing "es"/"s"), a small synonym table. The tests below describe
//! the behavior this function needs to have — they will fail against the current
//! placeholder implementation until you implement it.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    /// Input matches an existing known item exactly (case-insensitive).
    Exact(String),
    /// Input is judged to refer to an existing known item despite not matching exactly
    /// (typo, plural, etc). Carries the canonical name it resolved to.
    Fuzzy(String),
    /// No existing item is a good enough match; treat as a new canonical item.
    NoMatch,
}

/// `input` is what the user typed. `known` is the list of canonical names already in
/// the fridge (or a seed dictionary, your choice).
pub fn resolve_item_name(input: &str, known: &[String]) -> MatchResult {
    // Placeholder: exact case-insensitive match only. Everything else falls through
    // to NoMatch, which is wrong for typos/plurals — that's your implementation work.
    let normalized = input.trim().to_lowercase();
    for candidate in known {
        if candidate.to_lowercase() == normalized {
            return MatchResult::Exact(candidate.clone());
        }
    }
    MatchResult::NoMatch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["tomato".to_string(), "olive oil".to_string(), "milk".to_string()]
    }

    #[test]
    fn exact_match_case_insensitive() {
        assert_eq!(
            resolve_item_name("Tomato", &known()),
            MatchResult::Exact("tomato".to_string())
        );
    }

    #[test]
    fn plural_resolves_to_singular() {
        assert_eq!(
            resolve_item_name("tomatoes", &known()),
            MatchResult::Fuzzy("tomato".to_string())
        );
    }

    #[test]
    fn common_typo_resolves() {
        assert_eq!(
            resolve_item_name("tomatoe", &known()),
            MatchResult::Fuzzy("tomato".to_string())
        );
    }

    #[test]
    fn unrelated_word_is_no_match() {
        assert_eq!(resolve_item_name("bicycle", &known()), MatchResult::NoMatch);
    }

    #[test]
    fn multi_word_item_matches() {
        assert_eq!(
            resolve_item_name("olive oil", &known()),
            MatchResult::Exact("olive oil".to_string())
        );
    }
}
