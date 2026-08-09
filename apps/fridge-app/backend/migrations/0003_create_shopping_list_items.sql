CREATE TABLE shopping_list_items (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    quantity REAL NOT NULL DEFAULT 1,
    unit TEXT NOT NULL DEFAULT 'count',
    is_grocery INTEGER NOT NULL DEFAULT 1,
    added_manually INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'pending',
    foodkeeper_product_id INTEGER,
    added_at TEXT NOT NULL
);
