import { apiFetch } from "./apiClient";

export type ShoppingListStatus = "pending" | "purchased";

export type ShoppingListItem = {
  id: string;
  name: string;
  quantity: number;
  unit: string;
  is_grocery: boolean;
  added_manually: boolean;
  status: ShoppingListStatus;
  foodkeeper_product_id: number | null;
  added_at: string;
};

export type AddShoppingListItemInput = {
  name: string;
  quantity: number;
  unit: string;
  is_grocery: boolean;
  /** False when added by accepting a suggestion rather than typed in. Defaults to true. */
  added_manually?: boolean;
  foodkeeper_product_id?: number | null;
};

/** Why the backend's (currently unimplemented) `suggest_shopping_items` surfaced this item. */
export type ShoppingSuggestionReason = "frequently_purchased" | "expiring_replacement";

export type ShoppingSuggestion = {
  item_name: string;
  reason: ShoppingSuggestionReason;
};

export async function fetchShoppingList(): Promise<ShoppingListItem[]> {
  const res = await apiFetch(`/shopping-list`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch shopping list: ${res.status}`);
  return res.json();
}

export async function addShoppingListItem(
  input: AddShoppingListItemInput,
): Promise<ShoppingListItem> {
  const res = await apiFetch(`/shopping-list`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`Failed to add shopping list item: ${res.status}`);
  return res.json();
}

export async function removeShoppingListItem(id: string): Promise<void> {
  const res = await apiFetch(`/shopping-list/${id}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`Failed to remove shopping list item: ${res.status}`);
}

/**
 * Marks an item purchased. For grocery items the backend also folds it into the fridge
 * (same insert/merge path as adding directly), so it can show up there immediately.
 */
export async function markPurchased(id: string): Promise<ShoppingListItem> {
  const res = await apiFetch(`/shopping-list/${id}/purchase`, { method: "POST" });
  if (!res.ok) throw new Error(`Failed to mark item purchased: ${res.status}`);
  return res.json();
}

/** Always returns [] until `suggest_shopping_items` is implemented (see backend CLAUDE.md). */
export async function fetchShoppingSuggestions(): Promise<ShoppingSuggestion[]> {
  const res = await apiFetch(`/shopping-list/suggestions`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch suggestions: ${res.status}`);
  return res.json();
}
