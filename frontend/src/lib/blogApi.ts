import { apiBase, apiFetch } from "./apiClient";

export type BlogPost = {
  id: string;
  author_id: string;
  title: string;
  slug: string;
  /** Markdown source. Rendered by `app/blog/MarkdownBody.tsx`, never stored as HTML. */
  body: string;
  published: boolean;
  created_at: string;
  updated_at: string;
  /**
   * `"db"` for a post written here, `"file"` for one synced from `content/blog/*.md`.
   * File-sourced posts are read-only — the backend answers PATCH/DELETE on one with 409,
   * because the next sync would overwrite the edit from disk anyway.
   */
  source: BlogPostSource;
};

export type BlogPostSource = "db" | "file";

export type BlogSortOrder = "newest" | "oldest";

export type ListPostsOptions = {
  sort?: BlogSortOrder;
  q?: string;
};

/** What one run of the file sync changed. */
export type BlogSyncReport = {
  created: number;
  updated: number;
  deleted: number;
  skipped: number;
};

export type CreateBlogPostInput = {
  title: string;
  body: string;
  published: boolean;
};

export type UpdateBlogPostInput = Partial<CreateBlogPostInput>;

async function errorMessage(res: Response, fallback: string): Promise<string> {
  try {
    const body: { error?: string } = await res.json();
    return body.error ?? fallback;
  } catch {
    return fallback;
  }
}

/**
 * Every post the requester can see: published posts for a signed-out visitor or a
 * non-admin, drafts included for an admin. Uses a plain `fetch` rather than `apiFetch` —
 * this route works signed-out, so a missing cookie is the ordinary case, not a 401 to raise.
 */
export async function fetchPosts(options: ListPostsOptions = {}): Promise<BlogPost[]> {
  const params = new URLSearchParams();
  // Only send what was actually asked for: an empty `q=` is not a search for the empty
  // string, and omitting `sort` lets the backend's own default (newest) stand.
  if (options.sort) params.set("sort", options.sort);
  if (options.q?.trim()) params.set("q", options.q.trim());

  const query = params.toString();
  const res = await fetch(`${apiBase()}/blog/posts${query ? `?${query}` : ""}`, {
    credentials: "include",
    cache: "no-store",
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not load posts"));
  }
  return res.json();
}

/** A single published post by slug (or any post, if the requester is an admin). */
export async function fetchPostBySlug(slug: string): Promise<BlogPost | null> {
  const res = await fetch(`${apiBase()}/blog/posts/by-slug/${encodeURIComponent(slug)}`, {
    credentials: "include",
    cache: "no-store",
  });

  if (res.status === 404) return null;
  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not load that post"));
  }
  return res.json();
}

/** Admin-only: create a post. Rejects with the backend's message on a non-admin account. */
export async function createPost(input: CreateBlogPostInput): Promise<BlogPost> {
  const res = await apiFetch("/blog/posts", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not create post"));
  }
  return res.json();
}

/** Admin-only: partial update — only the fields present in `input` change. */
export async function updatePost(id: string, input: UpdateBlogPostInput): Promise<BlogPost> {
  const res = await apiFetch(`/blog/posts/${encodeURIComponent(id)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not update post"));
  }
  return res.json();
}

/** Admin-only: delete a post. */
export async function deletePost(id: string): Promise<void> {
  const res = await apiFetch(`/blog/posts/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not delete post"));
  }
}

/**
 * Admin-only: re-read `content/blog/` and reconcile it with the database.
 *
 * Uses `apiFetch` rather than the plain `fetch` the public read paths use — this one genuinely
 * requires a session, so a 401 should raise and redirect rather than being the ordinary
 * signed-out case.
 */
export async function syncBlogFiles(): Promise<BlogSyncReport> {
  const res = await apiFetch("/blog/sync", { method: "POST" });

  if (!res.ok) {
    throw new Error(await errorMessage(res, "Could not sync files"));
  }
  return res.json();
}
