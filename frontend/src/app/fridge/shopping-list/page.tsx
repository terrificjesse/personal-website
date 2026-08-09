"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import {
  addShoppingListItem,
  fetchShoppingList,
  fetchShoppingSuggestions,
  markPurchased,
  removeShoppingListItem,
  type AddShoppingListItemInput,
  type ShoppingListItem,
  type ShoppingSuggestion,
} from "@/lib/shoppingListApi";
import { AddShoppingItemForm } from "./AddShoppingItemForm";
import { SuggestedItemsPanel } from "./SuggestedItemsPanel";

export default function ShoppingListPage() {
  const [items, setItems] = useState<ShoppingListItem[]>([]);
  const [suggestions, setSuggestions] = useState<ShoppingSuggestion[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;

    Promise.all([fetchShoppingList(), fetchShoppingSuggestions()])
      .then(([listData, suggestionData]) => {
        if (cancelled) return;
        setItems(listData);
        setSuggestions(suggestionData);
        setError(null);
      })
      .catch(() => {
        if (cancelled) return;
        setError("Couldn't reach the fridge API. Is the backend running on :8080?");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [reloadKey]);

  async function handleAdd(input: AddShoppingListItemInput) {
    await addShoppingListItem(input);
    setReloadKey((key) => key + 1);
  }

  async function handleAcceptSuggestion(suggestion: ShoppingSuggestion) {
    await addShoppingListItem({
      name: suggestion.item_name,
      quantity: 1,
      unit: "count",
      is_grocery: true,
      added_manually: false,
    });
    setReloadKey((key) => key + 1);
  }

  async function handleRemove(id: string) {
    await removeShoppingListItem(id);
    setItems((prev) => prev.filter((item) => item.id !== id));
  }

  async function handlePurchase(id: string) {
    await markPurchased(id);
    setReloadKey((key) => key + 1);
  }

  const pending = items.filter((item) => item.status === "pending");

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Shopping list</h1>
      <p className="mt-1 text-sm opacity-70">
        <Link href="/fridge" className="underline underline-offset-4">
          Fridge
        </Link>
        {" · What to buy next."}
      </p>

      <div className="mt-6">
        <AddShoppingItemForm onAdd={handleAdd} />
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : pending.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">Nothing on the list.</p>
      ) : (
        <ul className="mt-6 divide-y divide-black/10 dark:divide-white/10">
          {pending.map((item) => (
            <li key={item.id} className="flex items-center justify-between gap-4 py-3">
              <div>
                <p className="font-medium capitalize">{item.name}</p>
                <p className="text-xs opacity-60">
                  {item.quantity} {item.unit}
                  {!item.is_grocery && " · non-grocery"}
                </p>
              </div>
              <div className="flex items-center gap-3">
                <button
                  onClick={() => handlePurchase(item.id)}
                  className="text-xs text-green-600 hover:underline"
                >
                  Purchased
                </button>
                <button
                  onClick={() => handleRemove(item.id)}
                  className="text-xs text-red-600 hover:underline"
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <h2 className="mt-10 text-sm font-semibold opacity-80">Suggested</h2>
      <SuggestedItemsPanel suggestions={suggestions} onAccept={handleAcceptSuggestion} />
    </div>
  );
}
