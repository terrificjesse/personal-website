
use serde_json::Value;

use crate::models::{Recipe, RecipeIngredient};

const MEALS_JSON: &str = include_str!("../data/themealdb/meals.json");

/// Maps appliances to their keywords
const APPLIANCE_KEYWORDS: &[(&str, &[&str])] = &[
    ("Oven", &["oven", "preheat"]),
    (
        "Stovetop",
        &[
            "skillet",
            "wok",
            "griddle",
            "saucepan",
            "frying pan",
            "stovetop",
            "hob",
            "saute",
            "sauté",
        ],
    ),
    ("Blender", &["blender"]),
    ("Food Processor", &["food processor"]),
    ("Microwave", &["microwave"]),
    ("Grill", &["grill", "barbecue", "bbq"]),
    (
        "Slow Cooker",
        &["slow cooker", "crockpot", "crock pot", "crock-pot"],
    ),
    (
        "Pressure Cooker",
        &["pressure cooker", "instant pot", "instapot"],
    ),
    (
        "Stand Mixer",
        &["stand mixer", "electric mixer", "hand mixer"],
    ),
    (
        "Deep Fryer",
        &["deep fry", "deep-fry", "deep frying", "deep fried"],
    ),
    ("Air Fryer", &["air fryer", "air-fryer"]),
];

/// Common condiments:
const PANTRY_STAPLE_KEYWORDS: &[&str] = &[
    "salt",
    "black pepper",
    "white pepper",
    "ground pepper",
    "peppercorn",
    "sugar",
    "flour",
    "oil",
    "vinegar",
    "baking powder",
    "baking soda",
    "cornstarch",
    "corn starch",
    "vanilla extract",
    "yeast",
    "cumin",
    "paprika",
    "cinnamon",
    "nutmeg",
    "oregano",
    "thyme",
    "garlic powder",
    "onion powder",
    "chili powder",
    "chilli powder",
    "cayenne",
    "turmeric",
    "bay leaf",
    "bay leaves",
    "curry powder",
    "ground ginger",
    "stock cube",
    "bouillon",
    "allspice",
    "cloves",
];

pub struct Catalog {
    recipes: Vec<Recipe>,
}

impl Catalog {
    /// Load the recipes from the catalog
    pub fn load() -> anyhow::Result<Self> {
        let raw: Vec<Value> = serde_json::from_str(MEALS_JSON)?;
        let recipes = raw.iter().filter_map(parse_recipe).collect();
        Ok(Self { recipes })
    }

    pub fn recipes(&self) -> &[Recipe] {
        &self.recipes
    }
}

/// Parses the recipe from the given JSON
fn parse_recipe(value: &Value) -> Option<Recipe> {
    let id = value.get("idMeal")?.as_str()?.to_string();
    let name = value.get("strMeal")?.as_str()?.to_string();
    let instructions = value
        .get("strInstructions")
        .and_then(Value::as_str)
        .unwrap_or("");
    let instructions_lower = instructions.to_lowercase();

    let country = value
        .get("strCountry")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let category = value
        .get("strCategory")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let image_url = value
        .get("strMealThumb")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut fridge_ingredients = Vec::new();
    let mut extra_ingredients = Vec::new();
    for i in 1..=20 {
        let ingredient_name = value
            .get(format!("strIngredient{i}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(ingredient_name) = ingredient_name else {
            continue;
        };
        let measure = value
            .get(format!("strMeasure{i}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string();

        let entry = RecipeIngredient {
            name: ingredient_name.to_string(),
            measure,
        };
        if is_pantry_staple(ingredient_name) {
            extra_ingredients.push(entry);
        } else {
            fridge_ingredients.push(entry);
        }
    }

    Some(Recipe {
        id,
        name,
        cuisine_tags: country.map(|c| vec![c.to_string()]).unwrap_or_default(),
        meal_type_tags: category.map(|c| vec![c.to_string()]).unwrap_or_default(),
        cook_time_minutes: None,
        required_appliances: derive_required_appliances(&instructions_lower),
        fridge_ingredients,
        extra_ingredients,
        image_url,
        instructions: instructions.trim().to_string(),
    })
}

/// Helper function that searches for pantry staple keywords
fn is_pantry_staple(ingredient_name: &str) -> bool {
    let lower = ingredient_name.to_lowercase();
    PANTRY_STAPLE_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
}

/// Helper function that searches for required appliance keywords
fn derive_required_appliances(instructions_lower: &str) -> Vec<String> {
    APPLIANCE_KEYWORDS
        .iter()
        .filter(|(_, triggers)| triggers.iter().any(|t| instructions_lower.contains(t)))
        .map(|(name, _)| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().expect("embedded themealdb JSON should parse")
    }

    #[test]
    fn parses_a_non_trivial_number_of_recipes() {
        assert!(catalog().recipes().len() > 500);
    }

    #[test]
    fn every_recipe_has_an_id_and_name() {
        assert!(
            catalog()
                .recipes()
                .iter()
                .all(|r| !r.id.is_empty() && !r.name.is_empty())
        );
    }

    #[test]
    fn cook_time_is_always_none() {
        assert!(
            catalog()
                .recipes()
                .iter()
                .all(|r| r.cook_time_minutes.is_none())
        );
    }

    #[test]
    fn every_recipe_has_non_empty_instructions() {
        assert!(
            catalog()
                .recipes()
                .iter()
                .all(|r| !r.instructions.is_empty())
        );
    }

    #[test]
    fn instructions_are_trimmed() {
        assert!(
            catalog()
                .recipes()
                .iter()
                .all(|r| r.instructions.trim() == r.instructions)
        );
    }

    #[test]
    fn oven_recipe_gets_oven_in_required_appliances() {
        let appliances =
            derive_required_appliances("preheat the oven to 350f and bake for 20 minutes");
        assert!(appliances.iter().any(|a| a == "Oven"));
    }

    #[test]
    fn recipe_mentioning_nothing_gets_no_appliances() {
        assert!(derive_required_appliances("chop the tomatoes and serve").is_empty());
    }

    #[test]
    fn bell_pepper_is_not_classified_as_a_pantry_staple() {
        assert!(!is_pantry_staple("Red Bell Pepper"));
        assert!(!is_pantry_staple("Green Pepper"));
    }

    #[test]
    fn black_pepper_is_classified_as_a_pantry_staple() {
        assert!(is_pantry_staple("Black Pepper"));
    }

    #[test]
    fn common_staples_are_classified_correctly() {
        for staple in [
            "Salt",
            "Plain Flour",
            "Olive Oil",
            "Ground Cumin",
            "Baking Powder",
        ] {
            assert!(
                is_pantry_staple(staple),
                "{staple} should be a pantry staple"
            );
        }
    }

    #[test]
    fn proteins_and_produce_are_not_pantry_staples() {
        for item in [
            "Chicken",
            "Milk",
            "Tomatoes",
            "Beef Mince",
            "Cheddar Cheese",
        ] {
            assert!(
                !is_pantry_staple(item),
                "{item} should not be a pantry staple"
            );
        }
    }

    #[test]
    fn every_recipe_ingredient_lands_in_exactly_one_bucket() {
        for recipe in catalog().recipes() {
            let fridge_names: Vec<_> = recipe.fridge_ingredients.iter().map(|i| &i.name).collect();
            let extra_names: Vec<_> = recipe.extra_ingredients.iter().map(|i| &i.name).collect();
            for name in &fridge_names {
                assert!(!extra_names.contains(name));
            }
        }
    }
}
