"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchMe, type AuthenticatedUser } from "@/lib/authApi";
import { createPost, deletePost, fetchPosts, updatePost, type BlogPost } from "@/lib/blogApi";
import { useApiError } from "@/lib/useApiError";
import { PostForm } from "./PostForm";

/**
 * The blog editor. Admin-only, but this page can only make an *optimistic* check — the same
 * limitation `proxy.ts` documents for the session cookie applies here to the `is_admin` flag:
 * only the backend can authoritatively refuse a non-admin, via `RequireAdmin` on every
 * write route. If the flag here ever disagreed with the backend, the backend wins and the
 * request 403s — this page just avoids showing the editor UI to someone it can't work for.
 */
export default function BlogAdminPage() {
  const [user, setUser] = useState<AuthenticatedUser | null | undefined>(undefined);
  const [posts, setPosts] = useState<BlogPost[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const handleApiError = useApiError();

  useEffect(() => {
    let cancelled = false;
    fetchMe().then((me) => {
      if (!cancelled) setUser(me);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!user?.is_admin) return;
    let cancelled = false;

    fetchPosts()
      .then((data) => {
        if (!cancelled) setPosts(data);
      })
      .catch((err) => {
        if (!cancelled) setError(handleApiError(err, "Couldn't load posts"));
      });

    return () => {
      cancelled = true;
    };
  }, [user, reloadKey, handleApiError]);

  if (user === undefined) {
    return <p className="mx-auto max-w-2xl px-4 py-10 text-sm opacity-60">Loading…</p>;
  }

  if (!user) {
    return (
      <div className="mx-auto max-w-2xl px-4 py-10">
        <p className="text-sm opacity-70">
          You need to{" "}
          <Link href="/login?next=/blog/admin" className="underline underline-offset-4">
            sign in
          </Link>{" "}
          to see this page.
        </p>
      </div>
    );
  }

  if (!user.is_admin) {
    return (
      <div className="mx-auto max-w-2xl px-4 py-10">
        <p className="text-sm opacity-70">Your account doesn&apos;t have access to the blog editor.</p>
      </div>
    );
  }

  async function handleCreate(input: Parameters<typeof createPost>[0]) {
    await createPost(input);
    setReloadKey((key) => key + 1);
  }

  async function handleUpdate(id: string, input: Parameters<typeof updatePost>[1]) {
    await updatePost(id, input);
    setEditingId(null);
    setReloadKey((key) => key + 1);
  }

  async function handleDelete(id: string) {
    if (!window.confirm("Delete this post? This can't be undone.")) return;
    await deletePost(id);
    setReloadKey((key) => key + 1);
  }

  return (
    <div className="mx-auto max-w-2xl px-4 py-10">
      <h1 className="text-xl font-semibold">Blog admin</h1>
      <p className="mt-1 text-sm opacity-70">
        <Link href="/blog" className="underline underline-offset-4">
          View the public blog
        </Link>
      </p>

      <div className="mt-6 rounded border border-black/10 dark:border-white/10 p-4">
        <h2 className="text-sm font-medium">New post</h2>
        <div className="mt-3">
          <PostForm submitLabel="Create" onSubmit={handleCreate} />
        </div>
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      <ul className="mt-6 divide-y divide-black/10 dark:divide-white/10">
        {posts.map((post) =>
          editingId === post.id ? (
            <li key={post.id} className="py-4">
              <PostForm
                initial={post}
                submitLabel="Save"
                onSubmit={(input) => handleUpdate(post.id, input)}
                onCancel={() => setEditingId(null)}
              />
            </li>
          ) : (
            <li key={post.id} className="flex items-center justify-between gap-4 py-4">
              <div>
                <p className="font-medium">{post.title}</p>
                <p className="text-xs opacity-60">
                  {new Date(post.updated_at).toLocaleDateString()}
                  {!post.published && " · Draft"}
                </p>
              </div>
              <div className="flex items-center gap-3 text-sm">
                <button onClick={() => setEditingId(post.id)} className="hover:underline">
                  Edit
                </button>
                <button onClick={() => handleDelete(post.id)} className="text-red-600 hover:underline">
                  Delete
                </button>
              </div>
            </li>
          ),
        )}
      </ul>
    </div>
  );
}
