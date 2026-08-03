"use client";

import { useState } from "react";
import type { AddItemInput } from "@/lib/fridgeApi";

export function AddItemForm({ onAdd }: { onAdd: (input: AddItemInput) => Promise<void> }) {
  const [name, setName] = useState("");
  const [quantity, setQuantity] = useState(1);
  const [unit, setUnit] = useState("count");
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setSubmitting(true);
    try {
      await onAdd({ name: name.trim(), quantity, unit });
      setName("");
      setQuantity(1);
      setUnit("count");
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
        <input
          id="item-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. tomato"
          className="rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
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
