"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchReviews, type ReviewWithRecipe } from "@/lib/reviewsApi";
import { useApiError } from "@/lib/useApiError";

export default function ReviewHistoryPage() {
  const [reviews, setReviews] = useState<ReviewWithRecipe[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const handleApiError = useApiError();

  useEffect(() => {
    fetchReviews()
      .then(setReviews)
      .catch((err) => setError(handleApiError(err, "Couldn't reach the fridge API. Is the backend running on :8080?")))
      .finally(() => setLoading(false));
  }, [handleApiError]);

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Review history</h1>
      <p className="mt-1 text-sm opacity-70">
        <Link href="/fridge/recipes" className="underline underline-offset-4">
          Recipes
        </Link>
        {" · Recipes you've cooked and rated."}
      </p>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : reviews.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">
          No reviews yet — mark a recipe cooked to leave one.
        </p>
      ) : (
        <ul className="mt-6 space-y-3">
          {reviews.map((review) => (
            <li
              key={review.id}
              className="flex gap-4 rounded border border-black/10 p-4 dark:border-white/10"
            >
              {review.recipe_image_url && (
                // eslint-disable-next-line @next/next/no-img-element -- external, unoptimized thumbnail from the vendored catalog
                <img
                  src={review.recipe_image_url}
                  alt=""
                  className="h-16 w-16 shrink-0 rounded object-cover"
                />
              )}
              <div className="min-w-0 flex-1">
                <p className="font-medium">{review.recipe_name}</p>
                <p className="mt-0.5 text-xs opacity-60">
                  {"★".repeat(review.rating) + "☆".repeat(5 - review.rating)}
                  {" · "}
                  {new Date(review.cooked_at).toLocaleDateString()}
                  {" · "}
                  {review.is_public ? "Public" : "Private"}
                </p>
                {review.notes && <p className="mt-1 text-xs opacity-80">{review.notes}</p>}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
