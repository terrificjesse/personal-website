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

## What it does

`browser.alarms` wakes the background page every few minutes. It fetches unacked events, shows
up to `maxNotificationsPerPoll` of them individually plus one summary for the rest, and acks
what it showed. **The server holds the dedup state** — an MV3 background page is killed between
alarms, so nothing it remembers can be trusted to still be there.

Clicking a notification opens the posting. The popup lists recent alerts and says when the last
check ran and how it went.

## Changing the backend origin

`host_permissions` in `manifest.json` lists the origins the extension may talk to. Point the
Backend URL at anything else and the fetch fails; **Test connection** says so explicitly rather
than looking like a dead backend. Add the origin there and reload the extension.

## Not in this phase

No content script, no autofill, no answer library (8f/8g), and nothing to do with Gmail
(8a–8d). The `email` alert kind in Settings is wired through and has no producer yet.
