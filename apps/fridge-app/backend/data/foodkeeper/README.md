# FoodKeeper data (USDA / FSIS)

Shelf-life reference data backing `src/expiration.rs`. Public domain (US government work).

## Provenance

Upstream: **FSIS FoodKeeper**, dataset version **`FMA-Data-v128.xlsx`**, published at
`https://www.fsis.usda.gov/shared/data/EN/foodkeeper.json`
(catalog page: <https://catalog.data.gov/dataset/fsis-foodkeeper-data>).

`products.csv` is a CSV rendering of that JSON's `Product` sheet, taken from the
[jelera/food-shelflife-db](https://github.com/jelera/food-shelflife-db) mirror because
fsis.usda.gov blocks scripted downloads (403 from Akamai; the file loads fine in a real
browser).

**The mirror was verified against the official feed, not trusted blindly.** Both sources
were reduced to the same normalized form and hashed:

| Check | Official v128 feed | `products.csv` |
|---|---|---|
| Product rows | 661 | 661 |
| Unique `Name` values | 468 | 468 |
| SHA-256 of sorted unique names | `78f06027da06…` | `78f06027da06…` |
| SHA-256 of 22 identity + shelf-life fields, all rows | `4c9ef6d9ccd8…` | `4c9ef6d9ccd8…` |

Identical. To re-verify after any upstream refresh, redo that comparison rather than
assuming the mirror stayed in sync — it is a third-party copy last updated in 2019, and it
matches today only because FoodKeeper itself has not changed.

`categories.csv` came from the official feed's `Category` sheet directly. The mirror's copy
of it **dropped the ID column**, which would have forced you to infer that row order equals
`Category_ID`. That inference happens to hold, but relying on it is a landmine, so the IDs
here are explicit.

The mirror dropped the **product `ID` column too** — `products.csv` starts at
`Category_ID`. The official feed has a per-product `ID` (1–661); these rows are in that
order, but nothing in the file says so. Since `Name` is not unique (see gotcha 6), your key
is the `(Name, Name_subtitle)` pair unless you re-import IDs from the official feed.

## Files

- `products.csv` — 661 rows, 37 columns. Unmodified from the mirror.
- `categories.csv` — 25 rows. `Category_ID` (1–25) → category + subcategory.

## Column semantics

Storage columns come in prefixed families: `Pantry`, `Refrigerate`, `Freeze`, each with
`_Min`, `_Max`, `_Metric`, and some with `_tips`.

The prefix that matters most here is **`DOP_`, which means _Date of Purchase_.** The
distinction is easy to get backwards and getting it backwards makes every estimate wrong:

- `DOP_Refrigerate_*` — keeps this long **from when you bought it**.
- `Refrigerate_*` (no prefix) — keeps this long **after the printed use-by date**.
- `*_After_Opening_*` — from when the package was opened.

Since `FridgeItem.added_at` is roughly purchase time, **`DOP_*` is the column family this
app wants.** Only one row (`Canadian bacon`, sliced) populates both, and it reads
`DOP=80 days` vs `after-date=10 days` — consistent with the two measuring from different
anchors.

## Gotchas found while profiling this data

1. **`_Metric` is a tagged union, not a unit.** Alongside `Days` / `Weeks` / `Months` /
   `Years` it also carries `Not Recommended` (66), `Package use-by date` (36),
   `Indefinitely` (13), `When Ripe` (12), and `Hours` (5). A `match` that only handles time
   units will silently mishandle ~130 values.
2. **`Year` and `Years` both appear** (2 vs 138). Same for casing drift elsewhere
   (`Barbecue Sauce` and `Barbecue sauce` are separate rows).
3. **The official data dictionary is incomplete.** It lists the metric vocabulary as
   "Days, Weeks, Months, When Ripe, Indefinitely, Not Recommended" — omitting `Years`,
   `Year`, `Hours`, and `Package use-by date`, all of which are present. Trust the data.
4. **Three `_Min` fields contain prose, not integers** — but all three are in
   `Refrigerate_After_Thawing_Min` (`"Or until best-by date."`, `"Depending on
   conditions."`). That column is the *only* source of non-integer numerics: drop it and
   every remaining `_Min`/`_Max` is a clean integer or empty, so they can be `Option<u32>`
   rather than `Option<String>`. Keep it and you need fallible parsing everywhere.
5. **The data is sparse.** Per-family row coverage: `DOP_Refrigerate` 235,
   `Refrigerate_After_Opening` 145, `Refrigerate` 129, `DOP_Pantry` 196, `DOP_Freeze` 196,
   `Freeze` 143, `Pantry` 98. **184 rows have no refrigerate data of any kind.** No single
   column answers the question; you need a fallback chain.
6. **`Name` is not a key.** 661 rows, 468 unique names — `Ham` appears 20 times, `Lamb` and
   `Pork` 12 each, `Beef` 10, `Cheese` 5, disambiguated only by `Name_subtitle`
   (`"hard such as cheddar, swiss, block parmesan"`). A user typing "ham" matches 20 rows
   with different shelf lives, so you need a rule for collapsing them.
7. **17 rows have trailing whitespace in `Name`** — `"Almonds "`, `"Turkey "`, `"Salami "`,
   `"Milk "` (×2), `"Ham "` (×2), and others. `Name_subtitle` has 2 more (`"canned "`,
   `"meat "`). This is why `Ham` counts as 20 rows after trimming but only 18 under exact
   comparison — and why an exact `==` lookup for `milk` silently misses 2 of its rows.
   Trim `Name` and `Name_subtitle` during normalization, before anything is indexed.
8. **`Keywords` is a ready-made synonym dictionary** — 660 of 661 rows have it, 2809 tokens,
   1453 unique, e.g. `Cheese,cheddar, swiss,parmesan`. This is also useful to
   `nlp::resolve_item_name`, not just to expiration. Note the inconsistent spacing after
   commas — trim every token.
