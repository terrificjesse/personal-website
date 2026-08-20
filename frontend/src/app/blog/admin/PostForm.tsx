"use client";

import { useState } from "react";
import type { CreateBlogPostInput } from "@/lib/blogApi";
import { MarkdownBody } from "../MarkdownBody";

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
  const [previewing, setPreviewing] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    // The textarea's `required` only guards the form while the textarea is mounted, and
    // Preview unmounts it — so an empty body could otherwise reach the backend and come back
    // as a bare 400.
    if (!body.trim()) {
      setPreviewing(false);
      setError("A post needs a body.");
      return;
    }

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
      <div>
        {/*
          Preview and the published page share `MarkdownBody`, so what's shown here can't
          drift from what a reader eventually sees.
        */}
        <div className="mb-2 flex items-center gap-1 text-xs">
          {(["write", "preview"] as const).map((mode) => {
            const active = (mode === "preview") === previewing;
            return (
              <button
                key={mode}
                type="button"
                onClick={() => setPreviewing(mode === "preview")}
                aria-pressed={active}
                className={
                  active
                    ? "rounded bg-black/10 dark:bg-white/15 px-2 py-1"
                    : "rounded px-2 py-1 opacity-60 hover:opacity-100"
                }
              >
                {mode === "write" ? "Write" : "Preview"}
              </button>
            );
          })}
        </div>

        {previewing ? (
          <div className="min-h-[11rem] rounded border border-black/10 dark:border-white/10 px-3 py-2">
            {body.trim() ? (
              <MarkdownBody>{body}</MarkdownBody>
            ) : (
              <p className="text-sm opacity-50">Nothing to preview yet.</p>
            )}
          </div>
        ) : (
          <textarea
            value={body}
            onChange={(e) => setBody(e.target.value)}
            placeholder="Write something… markdown is rendered on the published page."
            required
            rows={8}
            className="w-full rounded border border-black/10 dark:border-white/10 bg-transparent px-3 py-2 text-sm"
          />
        )}
      </div>
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
