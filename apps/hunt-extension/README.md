# Internship Hunt — Firefox extension

Phase 8e. Polls the site backend for `hunt_events` and raises a desktop notification for each
one. Rules and decisions live in `CLAUDE.md` next to this file; this is just how to run it.

## Load it

1. Start the backend (`cargo run` in `apps/fridge-app/backend`) and sign in to the site at
   `http://localhost:3000`, so the browser holds a `fridge_session` cookie.
2. Open `about:debugging#/runtime/this-firefox` in Firefox.
3. **Load Temporary Add-on…**, and pick this folder's `manifest.json`.
4. Open the extension's **Settings** and press **Test connection**. It reports which of the
   three failure modes you have, if any: no host permission, backend unreachable, or reachable
   but not signed in.

A temporary add-on is unloaded when Firefox closes. `browser_specific_settings.gecko.id` is
set, so its stored settings survive being loaded again.

**Step 4 is not optional, and it is not just a check.** Firefox MV3 treats `host_permissions`
as *optional*: listing the backend origin in `manifest.json` requests it, it does not grant it.
Until you grant it, every fetch fails with a bare `TypeError` that is indistinguishable from
the backend being down. **Test connection** asks for the grant — Firefox will show a permission
prompt the first time — because `permissions.request` only works from a user gesture, which an
alarm in the background page is not. You can also grant it by hand from the 🧩 Extensions
button → Internship Hunt → ⚙ → *Always Allow on localhost*.

(This is a real difference from Chrome, where declaring a host permission grants it at install.
The extension does not assume it; it asks.)

## What it does

`browser.alarms` wakes the background page every few minutes. It fetches unacked events, shows
up to `maxNotificationsPerPoll` of them individually plus one summary for the rest, and acks
what it showed. **The server holds the dedup state** — an MV3 background page is killed between
alarms, so nothing it remembers can be trusted to still be there.

Clicking a notification opens the posting. The popup lists recent alerts and says when the last
check ran and how it went.

## The backend has to allow the extension's origin

`credentials: "include"` makes every request a **credentialed cross-origin** request. The
browser discards the response unless it carries `Access-Control-Allow-Origin` naming the
caller — and the backend's `ALLOWED_ORIGINS` lists the *site's* origins, not the extension's.
A discarded response reaches JS as a bare `TypeError`, indistinguishable from a dead server,
which is why the extension used to report "can't reach localhost" while `curl` got a 200.

The extension's origin is `moz-extension://<uuid>`, where the UUID is generated per Firefox
profile. Find it either way:

- **Test connection** prints it in the failure message, or
- `about:debugging#/runtime/this-firefox` → the extension card → **Internal UUID**

Add it to `ALLOWED_ORIGINS` in `apps/fridge-app/backend/.env` and restart the backend:

    ALLOWED_ORIGINS=http://localhost:3000,http://127.0.0.1:3000,moz-extension://<uuid>

It is stable for that profile, so this is a one-time step — but it is per-profile, so a
different Firefox profile needs its own entry.

## Changing the backend origin

`host_permissions` in `manifest.json` lists the origins the extension may talk to. Point the
Backend URL at anything else and the fetch fails; **Test connection** says so explicitly rather
than looking like a dead backend. Add the origin there and reload the extension.

## Permissions, and why each one

| Permission | For |
|---|---|
| `alarms`, `notifications`, `storage` | the 8e alert poll |
| host `localhost:8080` | talking to the backend |
| `activeTab` + `scripting` | 8f autofill on pages that are *not* a known ATS |

`activeTab` grants access to **one tab, for one invocation**, when you click the toolbar
button. It is not standing access. Phase 7 found company-owned careers pages are most of the
corpus and cannot be enumerated into match patterns, so this is the only honest way to reach
them; the alternative is `<all_urls>`, which is permanent access to every page you visit, and
`CLAUDE.md` rules it out.

The known ATS hosts are handled separately, by declarative `content_scripts` on the six
hostnames `dedup::ats_identity` parses. The script only registers a listener on load — filling
happens on the popup's button and nowhere else.

**Changing permissions needs a fresh install, not a Reload.** Firefox computes a temporary
add-on's permission grants when it is installed, so after a manifest permission change,
`browser.scripting` and friends stay `undefined` until you **Remove** the add-on in
`about:debugging` and **Load Temporary Add-on…** again.

## Not in this phase

No content script, no autofill, no answer library (8f/8g), and nothing to do with Gmail
(8a–8d). The `email` alert kind in Settings is wired through and has no producer yet.

## Embedded application forms

A company careers page often *embeds* the ATS rather than linking to it:
`jumptrading.com/hr/job?gh_jid=…` renders an iframe pointing at `boards.greenhouse.io`, and the
form lives in that iframe, not in the page you are looking at.

Two settings make this work, and both default the wrong way for it:

- **`all_frames: true` on the content script.** The default is `false`, which loads it only in a
  tab's top frame — so an embedded Greenhouse form, whose URL matches perfectly, never gets it.
- **`allFrames: true` on `scripting.executeScript`** for the `activeTab` path. Note this cannot
  reach a *cross-origin* iframe on its own: `activeTab` grants the top-level origin only, which
  is why the declarative match above is what actually covers the embed.

If a form still will not fill, check whether it is behind an **Apply** button. Greenhouse embeds
commonly show the job description first and only render the form after a click, and a form that
is not in the DOM cannot be filled by anything.
