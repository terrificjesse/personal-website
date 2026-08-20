"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchPosts, type BlogPost, type BlogSortOrder } from "@/lib/blogApi";
import { fetchMe } from "@/lib/authApi";

/** How long to wait after the last keystroke before searching. */
const SEARCH_DEBOUNCE_MS = 250;

export default function BlogPage() {
  const [posts, setPosts] = useState<BlogPost[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sort, setSort] = useState<BlogSortOrder>("newest");
  // Two states rather than one: `query` is what's in the box and must update on every
  // keystroke, `debouncedQuery` is what's actually been asked for.
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [isAdmin, setIsAdmin] = useState(false);

  // Purely so an admin has a way *into* the editor — `/blog/admin` linked out to here but
  // nothing linked in, which left the editor reachable only by typing its URL. Optimistic in
  // the same sense as the admin page's own check: this decides whether to show a link, never
  // whether a write is allowed. `RequireAdmin` on the backend is what enforces.
  //
  // `fetchMe` returns null rather than throwing on 401, so this is safe on a page that has to
  // keep working signed out.
  useEffect(() => {
    let cancelled = false;

    fetchMe()
      .then((me) => {
        if (!cancelled) setIsAdmin(Boolean(me?.is_admin));
      })
      .catch(() => {
        // A signed-out visitor is the ordinary case here, not an error worth surfacing.
      });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(query), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    let cancelled = false;

    // Both controls feed one request. Sorting and searching compose in the backend's single
    // query, so there's nothing to merge or re-sort here — including across posts that came
    // from markdown files rather than the editor.
    fetchPosts({ sort, q: debouncedQuery })
      .then((data) => {
        // The cancelled flag matters more now than it did: typing fires overlapping requests,
        // and without it a slow early response could land after a newer one and win.
        if (cancelled) return;
        setPosts(data);
        setError(null);
      })
      .catch(() => {
        if (!cancelled) setError("Couldn't reach the blog API. Is the backend running on :8080?");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [sort, debouncedQuery]);

  const searching = debouncedQuery.trim().length > 0;

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-xl font-semibold">Blog</h1>
        {isAdmin && (
          <Link
            href="/blog/admin"
            className="shrink-0 rounded bg-black text-white dark:bg-white dark:text-black px-3 py-1.5 text-sm"
          >
            Write a post
          </Link>
        )}
      </div>

      <div className="mt-6 flex flex-wrap items-center gap-3">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search posts…"
          aria-label="Search posts"
          className="min-w-0 flex-1 rounded border border-black/10 dark:border-white/10 bg-transparent px-3 py-2 text-sm"
        />
        <div className="flex items-center gap-1 text-xs">
          {(["newest", "oldest"] as const).map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => setSort(option)}
              aria-pressed={sort === option}
              className={
                sort === option
                  ? "rounded bg-black text-white dark:bg-white dark:text-black px-2.5 py-1.5"
                  : "rounded px-2.5 py-1.5 opacity-60 hover:opacity-100"
              }
            >
              {option === "newest" ? "Newest" : "Oldest"}
            </button>
          ))}
        </div>
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : posts.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">
          {searching ? `No posts match “${debouncedQuery.trim()}”.` : "No posts yet."}
        </p>
      ) : (
        <ul className="mt-6 divide-y divide-black/10 dark:divide-white/10">
          {posts.map((post) => (
            <li key={post.id} className="py-4">
              <Link href={`/blog/${post.slug}`} className="font-medium underline underline-offset-4">
                {post.title}
              </Link>
              <p className="mt-1 text-xs opacity-60">
                {new Date(post.created_at).toLocaleDateString()}
                {!post.published && " · Draft"}
              </p>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
