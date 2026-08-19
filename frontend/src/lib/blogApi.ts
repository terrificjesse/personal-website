import { apiBase, apiFetch } from "./apiClient";

export type BlogPost = {
  id: string;
  author_id: string;
  title: string;
  slug: string;
  body: string;
  published: boolean;
  created_at: string;
  updated_at: string;
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
export async function fetchPosts(): Promise<BlogPost[]> {
  const res = await fetch(`${apiBase()}/blog/posts`, {
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
