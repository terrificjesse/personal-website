const API_BASE = process.env.NEXT_PUBLIC_FRIDGE_API_URL ?? "http://127.0.0.1:8080";

export type FridgeItem = {
  id: string;
  canonical_name: string;
  quantity: number;
  unit: string;
  added_at: string;
  estimated_expiration: string | null;
  foodkeeper_product_id: number | null;
};

export type AddItemInput = {
  name: string;
  quantity: number;
  unit: string;
  foodkeeper_product_id?: number | null;
};

/** Where a suggestion came from: already in the fridge, or the FoodKeeper catalog. */
export type SuggestionSource = "fridge" | "foodkeeper";

export type Suggestion = {
  name: string;
  source: SuggestionSource;
  foodkeeper_product_id: number | null;
  score: number;
};

/**
 * Ranked name suggestions for the add-item dropdown. An empty `query` returns recently
 * added fridge items instead of running the ranker.
 */
export async function fetchSuggestions(
  query: string,
  opts: { limit?: number; signal?: AbortSignal } = {},
): Promise<Suggestion[]> {
  const params = new URLSearchParams({ q: query });
  if (opts.limit !== undefined) params.set("limit", String(opts.limit));

  const res = await fetch(`${API_BASE}/items/suggest?${params}`, {
    cache: "no-store",
    signal: opts.signal,
  });
  if (!res.ok) throw new Error(`Failed to fetch suggestions: ${res.status}`);
  return res.json();
}

export async function fetchItems(): Promise<FridgeItem[]> {
  const res = await fetch(`${API_BASE}/items`, { cache: "no-store" });
  if (!res.ok) throw new Error(`Failed to fetch items: ${res.status}`);
  return res.json();
}

export async function addItem(input: AddItemInput): Promise<FridgeItem> {
  const res = await fetch(`${API_BASE}/items`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`Failed to add item: ${res.status}`);
  return res.json();
}

export async function removeItem(id: string): Promise<void> {
  const res = await fetch(`${API_BASE}/items/${id}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`Failed to remove item: ${res.status}`);
}
