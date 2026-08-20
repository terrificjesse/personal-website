# Blog posts as files

Any `.md` file in this directory becomes a blog post. Drop one in and it appears within a few
seconds — the backend re-checks this directory on a timer, so no restart and no button press.
It also syncs at startup, and **Re-sync files** in `/blog/admin` forces a check immediately.

`README.md` itself is skipped — it's documentation, not a post.

## Frontmatter

Every file starts with a fenced metadata block:

```markdown
---
title: What I Learned Building a Fridge App
date: 2026-08-19
published: true
---

The body starts here, and it's ordinary markdown.
```

| Key | Required | Notes |
|---|---|---|
| `title` | yes | Quote it if it contains a colon: `title: "Rust: a love story"` |
| `date` | yes | `YYYY-MM-DD`, or a full RFC 3339 timestamp. Becomes the post's `created_at`, which is what sorting uses. |
| `published` | no — defaults to `false` | `true`/`false`. Same opt-in default as the browser editor: publishing is always deliberate. |
| `slug` | no | Overrides the URL. By default the URL comes from the **filename**. |

**An unrecognized key is an error and the file is skipped**, with the reason logged. That's
deliberate: a misspelled `pubished: true` would otherwise leave the post a draft forever with
no symptom other than a post that never shows up.

## The URL comes from the filename

`my-first-post.md` publishes at `/blog/my-first-post`. It is deliberately **not** derived from
the title, so editing the title of a post that's already published doesn't break its URL —
the same rule `docs/BLOG.md` sets for browser-authored posts.

Renaming the file *does* change the URL. If you want to rename the file without moving the
post, pin the old URL with an explicit `slug:` first.

## Things that will get your file skipped

Each of these is logged with the filename and the reason:

- Missing or malformed frontmatter fences, or a missing `title` / `date`.
- An unknown frontmatter key.
- A slug already owned by a post written in the browser. The file loses — a URL that's already
  published keeps pointing where it did. Rename the file, or the post.
- Two files resolving to the same slug. The first wins.
- An empty body, or a title/body past the API's length limits.

## Deleting

The sync **mirrors** this directory: delete a file and its post goes with it, within the same
few seconds. This only ever touches file-sourced posts — a post written in the browser is never
removed by a sync.

## Editing

File-sourced posts are read-only in `/blog/admin`, and the API answers `PATCH`/`DELETE` on one
with **409**. The next sync would overwrite any edit from the file anyway; edit the file.

## Where the backend looks

`content/blog/` at the repo root, resolved relative to the backend's source directory so
`cargo run` works with no configuration. Set `BLOG_CONTENT_DIR` to point somewhere else, and
`BLOG_SYNC_INTERVAL_SECS` to change how often it's checked (`0` turns the watcher off).

Skipped files are reported in the backend log with the filename and the reason. If a post
isn't showing up, that log is the first place to look.
