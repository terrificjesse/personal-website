"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { fetchMe, type AuthenticatedUser } from "@/lib/authApi";
import {
  createPost,
  deletePost,
  BLOG_ADMIN_PAGE_LIMIT,
  fetchPosts,
  syncBlogFiles,
  updatePost,
  type BlogPost,
} from "@/lib/blogApi";
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
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);
  const [syncing, setSyncing] = useState(false);
  const [syncResult, setSyncResult] = useState<string | null>(null);
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

    // The editor asks for the largest page the backend allows rather than following cursors:
    // this list is a management view for one author, and `MAX_BLOG_PAGE_SIZE` is well past
    // what a personal blog reaches. `total` is still rendered, so if it is ever exceeded the
    // UI says so instead of quietly showing a prefix.
    fetchPosts({ limit: BLOG_ADMIN_PAGE_LIMIT })
      .then((page) => {
        if (cancelled) return;
        setPosts(page.posts);
        setTotal(page.total);
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

  async function handleSync() {
    setSyncing(true);
    setSyncResult(null);
    try {
      const report = await syncBlogFiles();
      setSyncResult(
        `${report.created} created, ${report.updated} updated, ${report.deleted} deleted, ` +
          `${report.skipped} skipped.` +
          (report.skipped > 0 ? " Check the backend log for why a file was skipped." : ""),
      );
      setReloadKey((key) => key + 1);
    } catch (err) {
      setError(handleApiError(err, "Couldn't sync files"));
    } finally {
      setSyncing(false);
    }
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
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="text-sm font-medium">Posts from files</h2>
            <p className="mt-1 text-xs opacity-60">
              <code>content/blog/*.md</code> syncs automatically every few seconds, and at
              startup. This forces it now — useful if you&apos;d rather not wait, or want to
              see why a file was skipped.
            </p>
          </div>
          <button
            onClick={handleSync}
            disabled={syncing}
            className="rounded border border-black/20 dark:border-white/20 px-3 py-1.5 text-sm disabled:opacity-50"
          >
            {syncing ? "Syncing…" : "Re-sync files"}
          </button>
        </div>
        {syncResult && <p className="mt-3 text-xs opacity-70">{syncResult}</p>}
      </div>

      <div className="mt-6 rounded border border-black/10 dark:border-white/10 p-4">
        <h2 className="text-sm font-medium">New post</h2>
        <div className="mt-3">
          <PostForm submitLabel="Create" onSubmit={handleCreate} />
        </div>
      </div>

      {error && <p className="mt-6 text-sm text-red-600">{error}</p>}

      {total > posts.length && (
        <p className="mt-6 text-xs text-amber-700 dark:text-amber-500">
          Showing {posts.length} of {total} posts. This view is capped at{" "}
          {BLOG_ADMIN_PAGE_LIMIT}; the rest are reachable through the public list.
        </p>
      )}

      <ul className="mt-6 divide-y divide-black/10 dark:divide-white/10">
        {posts.map((post) =>
          editingId === post.id && post.source !== "file" ? (
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
                  {post.source === "file" && " · From file"}
                </p>
              </div>
              {/*
                A file-sourced post has no Edit or Delete, because the next sync would
                overwrite either one from disk — the backend refuses both with 409 regardless,
                so this is the same optimistic-UI-over-real-enforcement split as `is_admin`.
              */}
              {post.source === "file" ? (
                <p className="shrink-0 text-xs opacity-50">Edit the markdown file</p>
              ) : (
                <div className="flex items-center gap-3 text-sm">
                  <button onClick={() => setEditingId(post.id)} className="hover:underline">
                    Edit
                  </button>
                  <button onClick={() => handleDelete(post.id)} className="text-red-600 hover:underline">
                    Delete
                  </button>
                </div>
              )}
            </li>
          ),
        )}
      </ul>
    </div>
  );
}
