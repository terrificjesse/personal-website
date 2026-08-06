"use client";

import { useState } from "react";
import type { AddItemInput, Suggestion } from "@/lib/fridgeApi";
import { ItemNameCombobox } from "./ItemNameCombobox";

export function AddItemForm({ onAdd }: { onAdd: (input: AddItemInput) => Promise<void> }) {
  const [name, setName] = useState("");
  const [quantity, setQuantity] = useState(1);
  const [unit, setUnit] = useState("count");
  const [submitting, setSubmitting] = useState(false);
  // Set only when the name came from a suggestion, so expiration estimation can look the
  // item up directly instead of re-matching the string.
  const [foodkeeperProductId, setFoodkeeperProductId] = useState<number | null>(null);

  function handleNameChange(next: string) {
    setName(next);
    // Editing the text after picking a suggestion breaks the link to that catalog entry.
    setFoodkeeperProductId(null);
  }

  function handleSelect(suggestion: Suggestion) {
    setName(suggestion.name);
    setFoodkeeperProductId(suggestion.foodkeeper_product_id);
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setSubmitting(true);
    try {
      await onAdd({
        name: name.trim(),
        quantity,
        unit,
        foodkeeper_product_id: foodkeeperProductId,
      });
      setName("");
      setQuantity(1);
      setUnit("count");
      setFoodkeeperProductId(null);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="flex flex-wrap items-end gap-3">
      <div className="flex flex-col">
        <label htmlFor="item-name" className="text-xs opacity-70">
          Item
        </label>
        <ItemNameCombobox
          inputId="item-name"
          value={name}
          onChange={handleNameChange}
          onSelect={handleSelect}
          placeholder="e.g. tomato"
        />
      </div>
      <div className="flex flex-col">
        <label htmlFor="item-quantity" className="text-xs opacity-70">
          Qty
        </label>
        <input
          id="item-quantity"
          type="number"
          min={0}
          step="any"
          value={quantity}
          onChange={(e) => setQuantity(Number(e.target.value))}
          className="w-20 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
      <div className="flex flex-col">
        <label htmlFor="item-unit" className="text-xs opacity-70">
          Unit
        </label>
        <input
          id="item-unit"
          value={unit}
          onChange={(e) => setUnit(e.target.value)}
          className="w-24 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
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
