-- Records which FoodKeeper product the user picked from the suggestion dropdown, so
-- expiration estimation can look up shelf life directly instead of re-matching the name.
-- NULL means the item was typed freehand and has no FoodKeeper counterpart.
ALTER TABLE fridge_items ADD COLUMN foodkeeper_product_id INTEGER;
