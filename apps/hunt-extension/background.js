/*
 * The background event page: poll the backend, raise notifications, ack what it showed.
 *
 * # This page is not running most of the time
 *
 * Firefox MV3 background pages are event pages. The browser starts one to deliver an event
 * and kills it again when it goes idle, so:
 *
 *   - every listener is registered synchronously at the top level, or the event that was
 *     supposed to wake the page arrives before anything is listening for it;
 *   - no state survives between wakes. Anything remembered in a module variable is gone.
 *
 * That second point is the whole reason `hunt_events.acked_at` lives on the server. A dedup
 * set held here would be empty on the next wake and every alert would fire again. Everything
 * this file writes to `browser.storage.local` is a cache for the popup — losing it costs a
 * nicer UI, never a duplicate or a dropped alert.
 *
 * # Notify, then ack — in that order
 *
 * At-least-once, deliberately. If the ack fails the event stays unacked and re-notifies on
 * the next poll; because the notification id IS the event id, Firefox replaces the existing
 * notification rather than stacking a second one. Acking first would invert the failure into
 * an alert that is silently lost, which is the expensive direction.
 */

const ALARM_NAME = "hunt-poll";
const SETTINGS_KEY = "settings";
const STATUS_KEY = "status";
const CACHE_KEY = "recentEvents";
const UNSEEN_KEY = "unseenCount";

const DEFAULT_SETTINGS = {
  backendUrl: "http://localhost:8080",
  // The bearer token from the site's Settings page. The session cookie cannot reach an
  // extension — `SameSite=Lax` means Firefox never attaches it to a request from a
  // `moz-extension://` page — so this is how the backend knows who we are.
  token: "",
  // Where "open the site" points. Used for a link only; nothing is fetched from it.
  siteUrl: "http://localhost:3000",
  // Firefox will not schedule an alarm more often than once a minute.
  pollMinutes: 5,
  kinds: { posting: true, email: true },
  // A cold database, or a week with Firefox closed, can leave hundreds of events waiting.
  // Past this many, the rest arrive as one summary notification instead of hundreds.
  maxNotificationsPerPoll: 3,
};

// How many events one poll asks for. The popup shows fewer; this is the notification budget.
const POLL_LIMIT = 50;

async function getSettings() {
  const stored = await browser.storage.local.get(SETTINGS_KEY);
  const saved = stored[SETTINGS_KEY] || {};
  return {
    ...DEFAULT_SETTINGS,
    ...saved,
    kinds: { ...DEFAULT_SETTINGS.kinds, ...(saved.kinds || {}) },
  };
}

/** Trailing slashes are the classic way to end up requesting `//hunt/events`. */
function apiUrl(settings, path) {
  return `${settings.backendUrl.replace(/\/+$/, "")}${path}`;
}

/**
 * Headers for an authenticated call.
 *
 * `credentials: "include"` is kept alongside the token so a future same-site caller still
 * works, but the token is what actually authenticates this extension.
 */
function authHeaders(settings) {
  return settings.token ? { Authorization: `Bearer ${settings.token}` } : {};
}

/** The origin of a configured URL, or null if it isn't a URL at all. */
function originOf(url) {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}

/**
 * One poll.
 *
 * Never throws: the caller is an alarm and a popup button, and neither has anywhere useful to
 * put an exception. Every outcome is recorded in `status` instead, because the three failure
 * modes are genuinely different and the popup has to be able to tell them apart:
 *
 *   unreachable    — the backend is not running, or the URL is wrong
 *   unauthenticated— it is running and does not know who we are: sign in on the site
 *   error          — it answered something unexpected
 *
 * Collapsing those into "no alerts" is the Phase 7 lesson exactly: a broken poll and a quiet
 * week must not look the same.
 */
async function poll() {
  const settings = await getSettings();

  // Firefox MV3 treats host_permissions as OPTIONAL — declaring the backend origin in the
  // manifest does not grant it, the user does, per origin. Checked before fetching because a
  // blocked request throws exactly the same bare TypeError as a dead server, so without this
  // the status would say "unreachable" and send you to restart a backend that is running.
  //
  // The grant can only be *asked for* from a user gesture, which an alarm is not. So this
  // reports the state and the options page does the asking.
  const origin = originOf(settings.backendUrl);
  if (origin && !(await browser.permissions.contains({ origins: [`${origin}/*`] }))) {
    return setStatus({ state: "unpermitted", detail: origin });
  }

  let response;

  try {
    response = await fetch(apiUrl(settings, `/hunt/events?limit=${POLL_LIMIT}`), {
      credentials: "include",
      headers: authHeaders(settings),
    });
  } catch (err) {
    return setStatus({ state: "unreachable", detail: String(err) });
  }

  if (response.status === 401) {
    // Distinguish "no token configured yet" from "the token was rejected". The first is
    // setup you have not done; the second is a token that was revoked or mistyped, and
    // sending you to the site to sign in again would be useless advice.
    return setStatus({
      state: settings.token ? "token-rejected" : "no-token",
    });
  }
  if (!response.ok) {
    return setStatus({ state: "error", detail: `HTTP ${response.status}` });
  }

  let payload;
  try {
    payload = await response.json();
  } catch (err) {
    return setStatus({ state: "error", detail: `unreadable response: ${err}` });
  }

  const events = (payload.events || []).filter((event) => settings.kinds[event.kind] !== false);
  await raise(events, settings);

  // `delivered`, not "how many are waiting": by the time this is written the batch has been
  // acked, so `payload.unacked_total` — read before the acks — would be a stale number
  // claiming work is outstanding that has just been done. The popup fetches the live count
  // itself; this only has to say what this poll did.
  return setStatus({ state: "ok", delivered: events.length });
}

/** Notify for these events, then ack them. Newest ends up on top of the stack. */
async function raise(events, settings) {
  if (events.length === 0) return;

  const individually = events.slice(0, Math.max(1, settings.maxNotificationsPerPoll));
  const summarized = events.length - individually.length;

  // The server returns newest first; show oldest of the batch first so the newest lands last
  // and is therefore the one on top.
  for (const event of [...individually].reverse()) {
    await notify(event);
  }
  if (summarized > 0) {
    await browser.notifications.create(`hunt-summary-${Date.now()}`, {
      type: "basic",
      title: `+${summarized} more posting${summarized === 1 ? "" : "s"}`,
      message: "Open the popup to see the rest.",
    });
  }

  await cacheForPopup(events);
  await bumpUnseen(events.length);

  // Ack everything that was shown, including the summarized ones — they are listed in the
  // popup, so they have been delivered.
  await Promise.all(events.map((event) => ack(event.id, settings)));
}

async function notify(event) {
  try {
    // The notification id IS the event id, so a re-notify after a failed ack replaces the
    // existing notification instead of stacking a duplicate.
    await browser.notifications.create(event.id, {
      type: "basic",
      title: event.title,
      message: event.body,
    });
  } catch (err) {
    // A notification we could not raise stays unacked below only if we let it. We do not:
    // failing to show one event must not block the rest of the batch.
    console.warn("hunt: could not raise a notification", err);
  }
}

async function ack(id, settings) {
  try {
    const response = await fetch(apiUrl(settings, `/hunt/events/${encodeURIComponent(id)}/ack`), {
      method: "POST",
      credentials: "include",
      headers: authHeaders(settings),
    });
    if (!response.ok && response.status !== 404) {
      console.warn(`hunt: ack for ${id} answered HTTP ${response.status}`);
    }
  } catch (err) {
    // Left unacked on purpose: it will be offered again on the next poll, which is the safe
    // direction. See the note at the top of this file.
    console.warn(`hunt: could not ack ${id}`, err);
  }
}

async function setStatus(status) {
  const record = { ...status, at: new Date().toISOString() };
  await browser.storage.local.set({ [STATUS_KEY]: record });
  await paintBadge();
  return record;
}

/**
 * Recent events, so the popup has something to draw before its own fetch returns and
 * something to draw at all when the backend is down. Capped, because this is a cache.
 */
async function cacheForPopup(events) {
  const stored = await browser.storage.local.get(CACHE_KEY);
  const merged = [...events, ...(stored[CACHE_KEY] || [])];
  const seen = new Set();
  const deduped = merged.filter((event) => {
    if (seen.has(event.id)) return false;
    seen.add(event.id);
    return true;
  });
  await browser.storage.local.set({ [CACHE_KEY]: deduped.slice(0, 50) });
}

async function bumpUnseen(count) {
  const stored = await browser.storage.local.get(UNSEEN_KEY);
  await browser.storage.local.set({ [UNSEEN_KEY]: (stored[UNSEEN_KEY] || 0) + count });
  await paintBadge();
}

/**
 * The badge counts alerts raised since you last opened the popup.
 *
 * Not the server's unacked count: everything gets acked within a second of being shown, so
 * that number is almost always 0. This one is a local convenience and is allowed to be — it
 * is the one piece of state here that genuinely belongs to this browser.
 */
async function paintBadge() {
  const stored = await browser.storage.local.get([UNSEEN_KEY, STATUS_KEY]);
  const unseen = stored[UNSEEN_KEY] || 0;
  const status = stored[STATUS_KEY] || {};

  if (status.state && status.state !== "ok") {
    await browser.action.setBadgeText({ text: "!" });
    await browser.action.setBadgeBackgroundColor({ color: "#b91c1c" });
    return;
  }
  await browser.action.setBadgeText({ text: unseen > 0 ? String(unseen) : "" });
  await browser.action.setBadgeBackgroundColor({ color: "#f59e0b" });
}

/**
 * Create the alarm only if it isn't already there.
 *
 * `alarms.create` with an existing name replaces it, which resets the countdown. Called on
 * every wake, that would push the next poll further away each time and the extension would
 * quietly stop polling.
 */
async function ensureAlarm() {
  const settings = await getSettings();
  const minutes = Math.max(1, Number(settings.pollMinutes) || DEFAULT_SETTINGS.pollMinutes);
  const existing = await browser.alarms.get(ALARM_NAME);
  if (existing && existing.periodInMinutes === minutes) return;
  await browser.alarms.create(ALARM_NAME, { periodInMinutes: minutes, delayInMinutes: 1 });
}

// --- listeners, all registered synchronously ------------------------------------------------

browser.runtime.onInstalled.addListener(() => {
  ensureAlarm().then(poll);
});

browser.runtime.onStartup.addListener(() => {
  ensureAlarm().then(poll);
});

browser.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM_NAME) poll();
});

browser.notifications.onClicked.addListener(async (notificationId) => {
  // The page that showed this notification is long dead, so the URL comes from the cache
  // rather than from memory. A cache miss opens nothing, which is the honest outcome — we
  // genuinely do not know where it pointed.
  const stored = await browser.storage.local.get(CACHE_KEY);
  const event = (stored[CACHE_KEY] || []).find((candidate) => candidate.id === notificationId);
  if (event && event.url) {
    await browser.tabs.create({ url: event.url });
  }
  await browser.notifications.clear(notificationId);
});

browser.runtime.onMessage.addListener((message) => {
  switch (message && message.type) {
    case "poll-now":
      return poll();
    case "settings-changed":
      return ensureAlarm().then(() => poll());
    case "popup-opened":
      return browser.storage.local
        .set({ [UNSEEN_KEY]: 0 })
        .then(paintBadge)
        .then(() => ({ ok: true }));
    default:
      return undefined;
  }
});
