//! FoodKeeper catalog — the dictionary of known food names that backs item suggestions.
//!
//! Read `data/foodkeeper/README.md` before touching this file. The parsing here is shaped
//! entirely by the data traps documented there; the relevant ones are called out inline.
//!
//! The CSV is embedded at compile time rather than read from disk so that the catalog works
//! regardless of the process's working directory (and is available in unit tests).

use std::collections::BTreeMap;

const PRODUCTS_CSV: &str = include_str!("../data/foodkeeper/products.csv");

/// One distinct food name, collapsed across every CSV row that shares it.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    /// Display name, trimmed. First-seen casing wins.
    pub name: String,
    /// Lowercased, trimmed, deduped `Keywords` values across all collapsed rows.
    /// These are alternate names ("spaghetti" for tomato sauce) and are worth matching
    /// against in addition to `name`.
    pub aliases: Vec<String>,
    /// Positional row ids (1-based) of every CSV row collapsed into this entry.
    ///
    /// README gotcha 6: `Name` is not unique — 661 rows, 468 distinct names, `Ham` alone
    /// appears 20 times with different shelf lives. Suggestions collapse them so the
    /// dropdown shows one "Ham"; disambiguating which variant the user meant is a problem
    /// for `expiration.rs`, which has `Name_subtitle` to work with.
    pub product_ids: Vec<i64>,
}

pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    /// Parses the embedded FoodKeeper CSV. Fails only if the embedded file is malformed,
    /// which would be a build-time problem, not a runtime one.
    pub fn load() -> anyhow::Result<Self> {
        let mut reader = csv::Reader::from_reader(PRODUCTS_CSV.as_bytes());

        let headers = reader.headers()?.clone();
        let column = |name: &str| -> anyhow::Result<usize> {
            headers
                .iter()
                .position(|h| h == name)
                .ok_or_else(|| anyhow::anyhow!("foodkeeper products.csv missing column {name}"))
        };
        let name_col = column("Name")?;
        let keywords_col = column("Keywords")?;

        // Keyed by lowercased name. This also folds together README gotcha 2's casing drift
        // ("Barbecue Sauce" vs "Barbecue sauce" are separate rows upstream).
        let mut collapsed: BTreeMap<String, CatalogEntry> = BTreeMap::new();

        for (index, record) in reader.records().enumerate() {
            let record = record?;

            // README gotcha 7: 17 rows have trailing whitespace in `Name`. Trim before
            // anything is keyed or indexed, or `milk` silently misses two of its rows.
            let name = record.get(name_col).unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }

            // README: the mirror dropped the product `ID` column. Rows are in the official
            // feed's id order (1-661), but nothing in the file asserts that, so this id is
            // positional by construction. Re-import ids from the official feed if that
            // assumption ever needs to be load-bearing.
            let product_id = (index + 1) as i64;

            let entry = collapsed
                .entry(name.to_lowercase())
                .or_insert_with(|| CatalogEntry {
                    name: name.to_string(),
                    aliases: Vec::new(),
                    product_ids: Vec::new(),
                });

            entry.product_ids.push(product_id);

            // README gotcha 8: `Keywords` is a ready-made synonym dictionary, with
            // inconsistent spacing after commas ("Cheese,cheddar, swiss,parmesan").
            let keywords = record.get(keywords_col).unwrap_or("");
            for keyword in keywords.split(',') {
                let keyword = keyword.trim().to_lowercase();
                if keyword.is_empty() || keyword == name.to_lowercase() {
                    continue;
                }
                if !entry.aliases.contains(&keyword) {
                    entry.aliases.push(keyword);
                }
            }
        }

        Ok(Self {
            entries: collapsed.into_values().collect(),
        })
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().expect("embedded foodkeeper CSV should parse")
    }

    #[test]
    fn collapses_to_distinct_names() {
        // 661 rows, 468 distinct names per the README — but that count is over raw `Name`
        // values, and we additionally fold by lowercase, so expect no more than that.
        let catalog = catalog();
        assert!(!catalog.entries().is_empty());
        assert!(catalog.entries().len() <= 468);
    }

    #[test]
    fn duplicate_names_keep_every_product_id() {
        let catalog = catalog();
        let ham = catalog
            .entries()
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("ham"))
            .expect("ham should be in the catalog");

        // README gotcha 6/7: 20 rows once trailing whitespace is trimmed.
        assert_eq!(ham.product_ids.len(), 20);
    }

    #[test]
    fn names_are_trimmed() {
        let catalog = catalog();
        assert!(catalog.entries().iter().all(|e| e.name.trim() == e.name));
    }

    #[test]
    fn aliases_are_trimmed_and_lowercased() {
        let catalog = catalog();
        assert!(
            catalog
                .entries()
                .iter()
                .flat_map(|e| &e.aliases)
                .all(|a| a.trim() == a && a.to_lowercase() == *a)
        );
    }

    #[test]
    fn keyword_synonyms_are_captured() {
        let catalog = catalog();
        let sauce = catalog
            .entries()
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("tomato sauce"))
            .expect("tomato sauce should be in the catalog");

        assert!(sauce.aliases.iter().any(|a| a.contains("spaghetti")));
    }
}
