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

const DEFAULTS = { backendUrl: "http://localhost:8080", siteUrl: "http://localhost:3000", token: "" };

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
      headers: config.token ? { Authorization: `Bearer ${config.token}` } : {},
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
    return say(
      `${config.backendUrl} didn't answer. If the site works in a tab, open Settings and ` +
        `press Test connection — it will print the exact fix.`,
      true,
    );
  }

  if (response.status === 401) {
    return say(
      config.token
        ? "Access token rejected — generate a new one on the site."
        : "No access token. Open Settings for how to get one.",
      true,
    );
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


// ------------------------------------------------------------------------------------------
// Autofill (Phase 8f)
// ------------------------------------------------------------------------------------------

/*
 * The button is the whole safety story. Nothing on an application page fills itself: the
 * content script registers a listener on load and waits for this click, which is rule 10's
 * "explicit user action, never on page load".
 *
 * The button only appears when the active tab actually has our content script in it — asking
 * you to press something that cannot work is its own small lie.
 */

const autofillEl = document.getElementById("autofill");
const fillButton = document.getElementById("fill");
const fillResultEl = document.getElementById("fillResult");

/** Whether the content script is already listening in this tab. */
async function isListening(tabId) {
  try {
    const pong = await browser.tabs.sendMessage(tabId, { type: "hunt-ping" });
    return Boolean(pong?.ready);
  } catch {
    // No content script there. Not an error — most tabs are not applications.
    return false;
  }
}

/** The active tab, whatever it is. */
async function activeTab() {
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  return tab?.id ? tab : null;
}

/**
 * Put the content script into a tab that does not already have it.
 *
 * This is the `activeTab` path, for the company careers pages Phase 7 found are most of the
 * corpus and which cannot be enumerated into match patterns. The permission is granted by
 * **your click on the toolbar button**, applies to that one tab, and lapses — it is not
 * standing access, and the extension cannot reach a page you have not asked it about.
 *
 * The declarative hosts do not come through here; they already have the script from load.
 */
async function injectInto(tabId) {
  await browser.scripting.executeScript({
    target: { tabId },
    // Order matters: fill.js reads the global fields.js defines.
    files: ["content/fields.js", "content/fill.js"],
  });
  return isListening(tabId);
}

function describe(report) {
  const parts = [];
  if (report.filled.length) {
    parts.push(`filled ${report.filled.length}: ${report.filled.map((f) => f.label).join(", ")}`);
  } else {
    parts.push("filled nothing");
  }
  if (report.alreadyFilled) parts.push(`${report.alreadyFilled} already had values`);
  // Named rather than counted: "3 refused" invites a shrug, "refused: password" does not.
  if (report.blocked.length) {
    parts.push(`refused ${report.blocked.length} sensitive field(s)`);
  }
  parts.push("nothing submitted — review before you send");
  return parts.join(" · ");
}

fillButton?.addEventListener("click", async () => {
  fillResultEl.textContent = "Filling…";
  try {
    const tab = await activeTab();
    if (!tab) {
      fillResultEl.textContent = "No active tab.";
      return;
    }

    // Inject on demand for anything that is not a known ATS. The click that opened this popup
    // is the gesture activeTab needs, so this is exactly the per-invocation access rule 10
    // asks for rather than a permission held permanently.
    if (!(await isListening(tab.id))) {
      try {
        if (!(await injectInto(tab.id))) {
          fillResultEl.textContent = "Could not read this page — it may block extensions.";
          return;
        }
      } catch (err) {
        // Firefox refuses about:, view-source:, addons.mozilla.org and similar outright.
        fillResultEl.textContent = `This page cannot be filled (${err.message}).`;
        return;
      }
    }
    const config = await settings();
    const base = config.backendUrl.replace(/\/+$/, "");
    const res = await fetch(`${base}/hunt/profile`, {
      credentials: "include",
      headers: config.token ? { Authorization: `Bearer ${config.token}` } : {},
    });
    if (!res.ok) {
      fillResultEl.textContent = "Could not load your CV details.";
      return;
    }
    const report = await browser.tabs.sendMessage(tab.id, {
      type: "hunt-fill",
      profile: await res.json(),
    });
    fillResultEl.textContent = describe(report);
  } catch (err) {
    fillResultEl.textContent = `Fill failed: ${err.message}`;
  }
});

/*
 * The button is always available now: on a known ATS the script is already there, and anywhere
 * else the click both grants access and injects it. Hiding it until a ping succeeded would
 * hide it on exactly the careers pages the activeTab path exists to reach.
 */
void (async () => {
  const tab = await activeTab();
  if (!tab) return;
  autofillEl.hidden = false;
  fillResultEl.textContent = (await isListening(tab.id))
    ? "Known application site."
    : "Other page — filling will ask this tab for access.";
})();
