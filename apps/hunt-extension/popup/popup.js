/*
 * The popup: recent alerts, and an honest statement of what the last poll actually did.
 *
 * It lists events including acked ones (`include_acked=true`). By the time you open this, the
 * background page has acked everything it notified you about — "ack" here means the client
 * took delivery, not that you read it — so a list of only unacked events would be empty on
 * every normal open.
 */

const SETTINGS_KEY = "settings";
const CACHE_KEY = "recentEvents";
const STATUS_KEY = "status";
const LIST_LIMIT = 25;

const DEFAULTS = { backendUrl: "http://localhost:8080", siteUrl: "http://localhost:3000" };

const statusEl = document.getElementById("status");
const listEl = document.getElementById("events");
const emptyEl = document.getElementById("empty");

async function settings() {
  const stored = await browser.storage.local.get(SETTINGS_KEY);
  return { ...DEFAULTS, ...(stored[SETTINGS_KEY] || {}) };
}

function relative(iso) {
  const seconds = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

/** Built with the DOM API rather than innerHTML: every string here came off the network. */
function render(events) {
  listEl.replaceChildren();
  emptyEl.hidden = events.length > 0;

  for (const event of events) {
    const item = document.createElement("li");
    const link = document.createElement("a");
    link.href = event.url || "#";
    link.target = "_blank";
    link.rel = "noreferrer";

    const title = document.createElement("div");
    title.className = "title";
    title.textContent = event.title;

    const body = document.createElement("div");
    body.className = "body";
    body.textContent = event.body;

    const meta = document.createElement("div");
    meta.className = "meta";
    meta.textContent = relative(event.created_at);

    link.append(title, body, meta);
    item.append(link);
    listEl.append(item);
  }
}

function say(text, bad = false) {
  statusEl.textContent = text;
  statusEl.classList.toggle("bad", bad);
}

/**
 * Draw the cache first so the popup is never blank, then replace it with live data.
 *
 * The cache is only ever a head start. If the fetch below disagrees with it, the fetch wins.
 */
async function load() {
  const cached = await browser.storage.local.get([CACHE_KEY, STATUS_KEY]);
  render(cached[CACHE_KEY] || []);

  const config = await settings();
  document.getElementById("site").href = config.siteUrl;

  let response;
  try {
    const base = config.backendUrl.replace(/\/+$/, "");
    response = await fetch(`${base}/hunt/events?include_acked=true&limit=${LIST_LIMIT}`, {
      credentials: "include",
    });
  } catch (err) {
    // Same trap as the options page: an ungranted host permission and a dead backend both
    // arrive here as a bare TypeError. Firefox MV3 does not grant host_permissions from the
    // manifest alone, so say which one it is rather than blaming the server.
    const origin = (() => {
      try {
        return new URL(config.backendUrl).origin;
      } catch {
        return null;
      }
    })();
    if (origin && !(await browser.permissions.contains({ origins: [`${origin}/*`] }))) {
      return say(`Access to ${origin} isn't granted — open Settings and press Test connection.`, true);
    }
    return say(`Can't reach ${config.backendUrl} — is the backend running?`, true);
  }

  if (response.status === 401) {
    return say("Not signed in. Open the site, log in, then check again.", true);
  }
  if (response.status === 403) {
    return say("The backend refused the request.", true);
  }
  if (!response.ok) {
    return say(`Backend answered HTTP ${response.status}.`, true);
  }

  const payload = await response.json();
  render(payload.events || []);

  const last = cached[STATUS_KEY];
  const when = last && last.at ? ` · last checked ${relative(last.at)}` : "";
  say(`${payload.unacked_total} waiting to be shown${when}`);
}

document.getElementById("check").addEventListener("click", async () => {
  say("Checking…");
  await browser.runtime.sendMessage({ type: "poll-now" });
  await load();
});

document.getElementById("options").addEventListener("click", (event) => {
  event.preventDefault();
  browser.runtime.openOptionsPage();
});

// Clears the badge: the count is "raised since you last looked", and you are looking.
browser.runtime.sendMessage({ type: "popup-opened" });
load();
