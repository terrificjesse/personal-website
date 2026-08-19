"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchPosts, type BlogPost } from "@/lib/blogApi";

export default function BlogPage() {
  const [posts, setPosts] = useState<BlogPost[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    fetchPosts()
      .then((data) => {
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
  }, []);

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Blog</h1>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : posts.length === 0 && !error ? (
        <p className="mt-6 text-sm opacity-60">No posts yet.</p>
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
