"use client";

import { useState } from "react";
import { MAX_NOTES_LENGTH, submitReview } from "@/lib/reviewsApi";

const RATINGS = [5, 4, 3, 2, 1];

export function ReviewForm({
  recipeId,
  onSubmitted,
}: {
  recipeId: string;
  onSubmitted: (rating: number) => void;
}) {
  const [rating, setRating] = useState(5);
  const [notes, setNotes] = useState("");
  // Defaults to private, matching the backend default — publishing is always a deliberate
  // choice, never something that happens because a control was left alone.
  const [isPublic, setIsPublic] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    try {
      await submitReview({
        recipe_id: recipeId,
        rating,
        notes: notes.trim() || undefined,
        is_public: isPublic,
      });
      onSubmitted(rating);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="mt-2 flex flex-wrap items-end gap-3 rounded border border-black/10 p-3 dark:border-white/10"
    >
      <div className="flex flex-col">
        <label htmlFor={`rating-${recipeId}`} className="text-xs opacity-70">
          Rating
        </label>
        <select
          id={`rating-${recipeId}`}
          value={rating}
          onChange={(e) => setRating(Number(e.target.value))}
          className="rounded border border-black/15 bg-transparent px-2 py-1.5 text-sm dark:border-white/20"
        >
          {RATINGS.map((r) => (
            <option key={r} value={r}>
              {"★".repeat(r) + "☆".repeat(5 - r)}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-1 flex-col">
        <label htmlFor={`notes-${recipeId}`} className="text-xs opacity-70">
          Notes (optional)
        </label>
        <input
          id={`notes-${recipeId}`}
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          maxLength={MAX_NOTES_LENGTH}
          placeholder="How did it turn out?"
          className="rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
        />
      </div>
      <label className="flex items-center gap-1.5 pb-1.5 text-xs opacity-70">
        <input
          type="checkbox"
          checked={isPublic}
          onChange={(e) => setIsPublic(e.target.checked)}
        />
        Share publicly
      </label>
      <button
        type="submit"
        disabled={submitting}
        className="rounded bg-foreground px-4 py-1.5 text-sm font-medium text-background disabled:opacity-50"
      >
        {submitting ? "Saving…" : "Save review"}
      </button>
    </form>
  );
}
