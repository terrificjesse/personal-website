-- One row per grocery acquisition event, written from a single call site
-- (`upsert_fridge_item` in src/routes/items.rs) regardless of whether the fridge row came
-- from the add-item form or from marking a shopping-list item purchased.
CREATE TABLE purchase_history (
    id TEXT PRIMARY KEY NOT NULL,
    item_name TEXT NOT NULL,
    quantity REAL NOT NULL,
    purchased_at TEXT NOT NULL
);
