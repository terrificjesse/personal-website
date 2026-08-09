"use client";

import { useState } from "react";
import type { AddShoppingListItemInput } from "@/lib/shoppingListApi";

export function AddShoppingItemForm({
  onAdd,
}: {
  onAdd: (input: AddShoppingListItemInput) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [quantity, setQuantity] = useState(1);
  const [unit, setUnit] = useState("count");
  const [isGrocery, setIsGrocery] = useState(true);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setSubmitting(true);
    try {
      await onAdd({ name: name.trim(), quantity, unit, is_grocery: isGrocery });
      setName("");
      setQuantity(1);
      setUnit("count");
      setIsGrocery(true);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-wrap items-end gap-3">
      <div className="flex flex-col">
        <label htmlFor="shopping-item-name" className="text-xs opacity-70">
          Item
        </label>
        <input
          id="shopping-item-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. paper towels"
          className="w-48 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
      <div className="flex flex-col">
        <label htmlFor="shopping-item-quantity" className="text-xs opacity-70">
          Qty
        </label>
        <input
          id="shopping-item-quantity"
          type="number"
          min={0}
          step="any"
          value={quantity}
          onChange={(e) => setQuantity(Number(e.target.value))}
          className="w-20 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
      <div className="flex flex-col">
        <label htmlFor="shopping-item-unit" className="text-xs opacity-70">
          Unit
        </label>
        <input
          id="shopping-item-unit"
          value={unit}
          onChange={(e) => setUnit(e.target.value)}
          className="w-24 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
      <label className="flex items-center gap-1.5 pb-1.5 text-xs opacity-70">
        <input
          type="checkbox"
          checked={isGrocery}
          onChange={(e) => setIsGrocery(e.target.checked)}
        />
        Grocery
      </label>
      <button
        type="submit"
        disabled={submitting || !name.trim()}
        className="rounded bg-foreground px-4 py-1.5 text-sm font-medium text-background disabled:opacity-50"
      >
        {submitting ? "Adding…" : "Add item"}
      </button>
    </form>
  );
}
