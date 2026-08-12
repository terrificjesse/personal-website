# TODO — deferred ideas

Things worth building that aren't worth building *now*. This is deliberately separate from
`PLAN.md`: that file is the committed phase-by-phase plan, this one is the parking lot for
ideas that came up mid-work and got consciously postponed.

Nothing here is a commitment. An entry earning its way out of this file is a decision to
make later, with whatever you know then.

## Format

Each entry keeps the same shape so this stays skimmable as it grows:

```
## <Short title>

**Status:** deferred — <what would make it worth revisiting>
**Area:** <file or subsystem>
**Added:** <YYYY-MM-DD>

<What it is, in a sentence or two.>

**Why not now:** <the reason it was postponed — the most important field, since future-you
won't remember the tradeoff that made this a "later">

**Sketch:** <rough approach, so you don't re-derive it from scratch>

**Watch out for:** <known traps, cost, or interactions>
```

---

## Out-of-order token matching

**Status:** deferred — revisit if real queries start missing, or after the fuzzy tier lands
**Area:** `apps/fridge-app/backend/src/nlp.rs`, prefix/scoring tiers
**Added:** 2026-08-07

Match a query's tokens against a candidate's tokens regardless of the order they're typed
in, so `milk almond` finds "Almond milk" and `oil olive` finds "Olive oil". Today both
queries return nothing: whole-string prefix fails because the string doesn't start that
way, and any-token prefix fails because a multi-word query can never prefix-match a single
token.

**Why not now:** people type left to right, so it's rare in practice. Whole-string prefix
already covers in-order multi-word queries for free (`almond mi` *is* a prefix of
`almond milk`), which is the overwhelmingly common case. This tier only earns its keep for
genuinely reordered input, and it's the most expensive and false-positive-prone of the
prefix variants — bad ratio while the cheap tiers aren't finished yet.

**Sketch:** split both query and candidate on whitespace. Require *every* query token to
prefix-match *some* candidate token, each candidate token consumed at most once. Score
below the in-order prefix sub-band, since word order is real evidence about intent and
throwing it away should cost something. Aggregate by total matched length over candidate
length, consistent with how coverage is measured elsewhere.

**Watch out for:**
- Cost is O(query_tokens x candidate_tokens) per candidate — the only tier that isn't
  linear in the candidate. Fine at 466 entries, worth remembering if the catalog grows.
- Long multi-token catalog names ("Roasted nuts (peanuts, cashews, almonds)") have many
  tokens and will match loosely. Consider requiring a minimum coverage before accepting.
- Needs a clear tie-break against the plain prefix tiers, or an out-of-order match can
  outrank an in-order one and the ranking stops being explainable.
- `rapidfuzz`'s `token_set_ratio` implements this class of matching if adopting beats
  building — worth a look before writing it by hand.

---

## Measure/quantity-aware ingredient matching

**Status:** deferred — revisit if presence-only matching starts recommending recipes you
don't actually have enough of a matched ingredient for
**Area:** `apps/fridge-app/backend/src/recommend_recipes.rs` (and by extension
`RecipeIngredient.measure`, `FridgeItem.unit`)
**Added:** 2026-08-12

`recommend_recipes` matches on ingredient *name* only — a recipe needing "2 cups flour"
counts as a match if you have any flour at all, even a teaspoon's worth. There's no check
that you have *enough* of a matched ingredient.

**Why not now:** solving it properly needs a lot more than this function. `RecipeIngredient.
measure` (from TheMealDB) and `FridgeItem.unit` (user-typed) are both unparsed free text on
either side of the comparison — before any conversion, you'd need two independent free-text
parsers to agree. And count-based ingredients ("3 tomatoes") can't be converted to
volume/weight ("2 cups") without a size/density reference table per ingredient — FoodKeeper
doesn't have that data (it's shelf-life only); it'd need a new source like USDA FoodData
Central. That's a data-source decision on the scale of the recipe-catalog one, not a tweak
to this function.

**Sketch:** if it's ever worth doing — parse both measure strings into (quantity, unit)
pairs; only attempt a quantity comparison when the units already match or are trivially
convertible (volume↔volume, mass↔mass); fall back to presence-only whenever units differ
across dimensions (count vs. volume) or don't parse. A partial version — same-unit-only
comparison, no cross-dimension conversion — is cheaper than the full problem and worth
considering as a first cut before reaching for a full reference table.

**Watch out for:**
- Free-text measure parsing is its own fragile subproblem (fractions, ranges, "a pinch",
  "to taste") independent of conversion.
- No general solution exists for count↔volume/mass without per-ingredient size data.
- Don't let this quietly expand `recommend_recipes`'s scope — it's a separate feature with
  its own data-source question, same shape as the recipe-catalog decision itself.

---

## Singular/plural exact-match misses

**Status:** deferred — revisit if recipe matches feel sparse, or a recipe you clearly have
the ingredients for doesn't show up
**Area:** `apps/fridge-app/backend/src/recommend_recipes.rs` (ingredient-name comparison
against `fridge`/`shopping_list`)
**Added:** 2026-08-12

`recommend_recipes` matches ingredient names by exact string equality (after lowercasing) —
a fridge item's `canonical_name` has to match a recipe ingredient's `name` character for
character. TheMealDB doesn't normalize singular vs. plural form across recipes, so a fridge
item typed as "tomatoes" only matches recipes whose ingredient list also says "Tomatoes";
recipes using singular "Tomato" are silently invisible to it, even though it's the same
ingredient. Measured against the real catalog: "tomatoes" matches 21 recipes exactly, but
singular "tomato" appears in 47 — more than double are missed. Same pattern for "eggs"
(109) vs. "egg" (127).

**Why not now:** this is the recipe-matching analog of the singular/plural problem `nlp.rs`
already solves for fridge-item suggestions (see its `plural_matches_singular_name` test and
module doc) — the fix likely wants the same kind of approach, not a bespoke one, and pulling
that logic into recipe matching is more than a one-line tweak. It's also low-severity on its
own: it never produces a wrong result, only an incomplete list, and presence-only matching
here is already a known simplification (see the quantity/measure entry above).

**Sketch:** cheapest fix is a small stemming step before comparison — strip a trailing "s"
(or "es") from both sides before checking set membership, the same idea `nlp.rs` already
applies. A denser fix would route ingredient-name comparison through the same fuzzy/alias
matching `nlp.rs` uses for the add-item flow, but that's a bigger dependency to take on just
for this one comparison.

**Watch out for:**
- Naive trailing-"s" stripping breaks on words that are plural without an "s" ("leaves") or
  singular words that end in "s" ("hummus", "molasses") — worth checking against the real
  ingredient-name list before trusting a blanket rule.
- Don't let this become a full fuzzy-matching pass on ingredient names — that's a much
  bigger scope change (see `nlp.rs`'s own tier design for how much surface area "fuzzy"
  implies) for a problem that's currently just "missing some matches," not "matching the
  wrong thing."
