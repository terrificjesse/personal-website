CREATE TABLE fridge_items (
    id TEXT PRIMARY KEY NOT NULL,
    canonical_name TEXT NOT NULL,
    quantity REAL NOT NULL DEFAULT 1,
    unit TEXT NOT NULL DEFAULT 'count',
    added_at TEXT NOT NULL,
    estimated_expiration TEXT
);
