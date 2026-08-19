"use client";

import { useState } from "react";
import type { CreateBlogPostInput } from "@/lib/blogApi";

type PostFormProps = {
  initial?: Partial<CreateBlogPostInput>;
  submitLabel: string;
  onSubmit: (input: CreateBlogPostInput) => Promise<void>;
  onCancel?: () => void;
};

export function PostForm({ initial, submitLabel, onSubmit, onCancel }: PostFormProps) {
  const [title, setTitle] = useState(initial?.title ?? "");
  const [body, setBody] = useState(initial?.body ?? "");
  const [published, setPublished] = useState(initial?.published ?? false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit({ title, body, published });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Something went wrong");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3">
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="Title"
        required
        className="w-full rounded border border-black/10 dark:border-white/10 bg-transparent px-3 py-2 text-sm"
      />
      <textarea
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="Write something…"
        required
        rows={8}
        className="w-full rounded border border-black/10 dark:border-white/10 bg-transparent px-3 py-2 text-sm"
      />
      <label className="flex items-center gap-2 text-sm opacity-80">
        <input
          type="checkbox"
          checked={published}
          onChange={(e) => setPublished(e.target.checked)}
        />
        Published
      </label>

      {error && <p className="text-sm text-red-600">{error}</p>}

      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={submitting}
          className="rounded bg-black text-white dark:bg-white dark:text-black px-3 py-1.5 text-sm disabled:opacity-50"
        >
          {submitting ? "Saving…" : submitLabel}
        </button>
        {onCancel && (
          <button
            type="button"
            onClick={onCancel}
            className="text-sm opacity-70 hover:opacity-100"
          >
            Cancel
          </button>
        )}
      </div>
    </form>
  );
}
