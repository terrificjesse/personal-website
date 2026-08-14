"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { addItem, fetchItems, removeItem, type AddItemInput, type FridgeItem } from "@/lib/fridgeApi";
import { AddItemForm } from "./AddItemForm";
import { ExpirationBadge } from "./ExpirationBadge";
import { GroceryListPopup } from "./GroceryListPopup";
import { useApiError } from "@/lib/useApiError";

export default function FridgePage() {
  const [items, setItems] = useState<FridgeItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const handleApiError = useApiError();

  useEffect(() => {
    let cancelled = false;

    fetchItems()
      .then((data) => {
        if (cancelled) return;
        setItems(data);
        setError(null);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(handleApiError(err, "Couldn't reach the fridge API. Is the backend running on :8080?"));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [reloadKey, handleApiError]);

  async function handleAdd(input: AddItemInput) {
    await addItem(input);
    setReloadKey((key) => key + 1);
  }

  async function handleRemove(id: string) {
    await removeItem(id);
    setItems((prev) => prev.filter((item) => item.id !== id));
  }

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Fridge</h1>
      <p className="mt-1 text-sm opacity-70">
        {"What's currently in the fridge. "}
        <Link href="/fridge/shopping-list" className="underline underline-offset-4">
          Shopping list
        </Link>
        {" · "}
        <Link href="/fridge/recipes" className="underline underline-offset-4">
          Recipes
        </Link>
      </p>

      <div className="mt-6">
        <AddItemForm onAdd={handleAdd} />
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : items.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">Nothing in the fridge yet.</p>
      ) : (
        <ul className="mt-6 divide-y divide-black/10 dark:divide-white/10">
          {items.map((item) => (
            <li key={item.id} className="flex items-center justify-between gap-4 py-3">
              <div>
                <p className="font-medium capitalize">{item.canonical_name}</p>
                <p className="text-xs opacity-60">
                  {item.quantity} {item.unit}
                </p>
              </div>
              <div className="flex items-center gap-3">
                <ExpirationBadge estimatedExpiration={item.estimated_expiration} />
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

      <GroceryListPopup />
    </div>
  );
}
