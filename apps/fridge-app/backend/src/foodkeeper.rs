
use std::collections::BTreeMap;

const PRODUCTS_CSV: &str = include_str!("../data/foodkeeper/products.csv");

/// Struct that manages the data fields in a FoodKeeper catalog entry
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub name: String,
    pub name_lower: String,
    pub aliases: Vec<String>,
    pub product_ids: Vec<i64>,
}

pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    /// Loading the dataset
    pub fn load() -> anyhow::Result<Self> {
        let mut reader = csv::Reader::from_reader(PRODUCTS_CSV.as_bytes());

        // Finds the indices of different columns with a matching header name
        let headers = reader.headers()?.clone();
        let column = |name: &str| -> anyhow::Result<usize> {
            headers
                .iter()
                .position(|h| h == name)
                .ok_or_else(|| anyhow::anyhow!("foodkeeper products.csv missing column {name}"))
        };
        let name_col = column("Name")?;
        let keywords_col = column("Keywords")?;

        // Collapses rows with duplicate names into a single entry
        let mut collapsed: BTreeMap<String, CatalogEntry> = BTreeMap::new();

        for (index, record) in reader.records().enumerate() {
            let record = record?;

            let name = record.get(name_col).unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }

            let product_id = (index + 1) as i64;

            let entry = collapsed
                .entry(name.to_lowercase())
                .or_insert_with(|| CatalogEntry {
                    name: name.to_string(),
                    name_lower: name.to_lowercase(),
                    aliases: Vec::new(),
                    product_ids: Vec::new(),
                });

            entry.product_ids.push(product_id);

            // Combines all of the keywords into a shared keyword bank
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
