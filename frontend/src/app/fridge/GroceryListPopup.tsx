"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchShoppingList, type ShoppingListItem } from "@/lib/shoppingListApi";

/** A quick-glance sticky note for the shopping list, without leaving the fridge tab. */
export function GroceryListPopup() {
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<ShoppingListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    setLoading(true);

    fetchShoppingList()
      .then((data) => {
        if (cancelled) return;
        setItems(data);
        setError(null);
      })
      .catch(() => {
        if (cancelled) return;
        setError("Couldn't load the shopping list.");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [open]);

  const pending = items.filter((item) => item.status === "pending");

  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col items-end gap-2">
      {open && (
        <div className="w-64 -rotate-1 rounded-sm bg-yellow-100 p-4 text-sm text-yellow-950 shadow-xl dark:bg-yellow-200">
          <p className="font-semibold">Grocery list</p>

          {loading ? (
            <p className="mt-2 opacity-70">Loading…</p>
          ) : error ? (
            <p className="mt-2 text-red-700">{error}</p>
          ) : pending.length === 0 ? (
            <p className="mt-2 opacity-70">Nothing on the list.</p>
          ) : (
            <ul className="mt-2 space-y-1">
              {pending.map((item) => (
                <li key={item.id} className="capitalize">
                  {item.name}{" "}
                  <span className="opacity-60">
                    · {item.quantity} {item.unit}
                  </span>
                </li>
              ))}
            </ul>
          )}

          <Link
            href="/fridge/shopping-list"
            className="mt-3 inline-block text-xs underline underline-offset-4"
          >
            Open full list
          </Link>
        </div>
      )}

      <button
        onClick={() => setOpen((v) => !v)}
        className="rounded-full bg-yellow-300 px-4 py-2 text-sm font-medium shadow-lg hover:bg-yellow-400 dark:bg-yellow-400 dark:hover:bg-yellow-500"
      >
        {open ? "Close list" : "📝 List"}
      </button>
    </div>
  );
}
