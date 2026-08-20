"use client";

import { use, useEffect, useState } from "react";
import Link from "next/link";
import { fetchPostBySlug, type BlogPost } from "@/lib/blogApi";
import { MarkdownBody } from "../MarkdownBody";

export default function BlogPostPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = use(params);
  const [post, setPost] = useState<BlogPost | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    fetchPostBySlug(slug)
      .then((data) => {
        if (!cancelled) setPost(data);
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
  }, [slug]);

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <Link href="/blog" className="text-sm opacity-70 hover:opacity-100 underline underline-offset-4">
        ← Blog
      </Link>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {loading ? (
        <p className="mt-6 text-sm opacity-60">Loading…</p>
      ) : !post ? (
        !error && <p className="mt-6 text-sm opacity-60">That post doesn&apos;t exist.</p>
      ) : (
        <article className="mt-6">
          <h1 className="text-2xl font-semibold">{post.title}</h1>
          <p className="mt-1 text-xs opacity-60">
            {new Date(post.created_at).toLocaleDateString()}
            {!post.published && " · Draft"}
          </p>
          <div className="mt-6">
            <MarkdownBody>{post.body}</MarkdownBody>
          </div>
        </article>
      )}
    </div>
  );
}
