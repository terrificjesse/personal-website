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
  const results = await browser.scripting.executeScript({
    // Every frame, because the form is very often not in the top one. A Greenhouse "embed" on
    // a company careers page — jumptrading.com/hr/job?gh_jid=… is exactly this — is an iframe
    // pointing at boards.greenhouse.io, and the top frame holds no form at all. Injecting only
    // there succeeds and fills nothing, which is the worst kind of working.
    //
    // Frames with no fillable fields stay silent (see fill.js), so the frame that answers is
    // the one holding the form.
    target: { tabId, allFrames: true },
    // Order matters: fill.js reads the global fields.js defines.
    //
    // LEADING SLASHES ARE LOAD-BEARING. Firefox resolves these relative to the CALLING page,
    // which is popup/popup.html — bare paths became `popup/content/fields.js` and failed with
    // "Unable to load script". Chrome resolves from the extension root, so every example
    // written against Chrome omits the slash and works there.
    files: ["/content/fields.js", "/content/fill.js"],
  });

  // executeScript resolves even when the injected code threw — the failure is reported per
  // frame in the result, not as a rejection. Ignoring that turns a real error into a silent
  // "the page must be blocking us", which is a guess dressed as an explanation.
  const failed = (results || []).find((result) => result?.error);
  if (failed) {
    const detail = failed.error?.message || String(failed.error);
    throw new Error(`the injected script threw: ${detail}`);
  }

  // A short retry rather than a single immediate ping. The listener registers at the end of
  // fill.js and the message port is set up asynchronously, so one ping can lose a race that
  // the very next one wins.
  for (let attempt = 0; attempt < 6; attempt += 1) {
    if (await isListening(tabId)) return true;
    await new Promise((resolve) => setTimeout(resolve, 120));
  }
  return false;
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
          const where = (() => {
            try {
              return new URL(tab.url || "").origin;
            } catch {
              return "this page";
            }
          })();
          fillResultEl.textContent =
            `Injected into ${where} but it never answered. If this is a PDF, a frame-heavy ` +
            `page, or a restricted Firefox page, that is expected.`;
          return;
        }
      } catch (err) {
        // Report what the RUNNING extension actually has, not what the manifest on disk says.
        // A temporary add-on keeps the permission set it was installed with, so after a
        // manifest change the two disagree — and "browser.scripting is undefined" looks like
        // a browser problem when it is really a stale install. Putting the running permission
        // list in the message turns a guess into a reading.
        const granted = browser.runtime.getManifest().permissions || [];
        const hint = granted.includes("scripting")
          ? "the running extension does have the scripting permission, so this is not a stale install"
          : "the running extension does NOT have the scripting permission — remove it in " +
            "about:debugging and Load Temporary Add-on… again; Reload keeps the old grants";
        fillResultEl.textContent =
          `This page cannot be filled (${err.message}). ` +
          `Running permissions: [${granted.join(", ")}] — ${hint}.`;
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


/*
 * "Track this application" — one click, never automatic.
 *
 * Filling a form is not applying. The extension has no way to know whether you pressed submit,
 * and inventing an `internship_applications` row for a form you abandoned would corrupt the one
 * table Phase 7 built to be trustworthy: `applied_at` is NOT NULL and means *you applied*.
 * So this is offered and never assumed, which is also why it appears after a fill rather than
 * as part of one.
 *
 * This is the legitimate way to create a tracker row, and it does not contradict the rule that
 * `Hunt/Outreach` must never create one — there, nobody applied. It matters beyond convenience:
 * an email can only be matched to an application that exists, so the rows this creates are what
 * make the inbox agent work later.
 */

const trackButton = document.getElementById("track");
const trackResultEl = document.getElementById("trackResult");

/** What the backend and the page each think this application is. */
async function identifyPage(tabId) {
  const config = await settings();
  const base = config.backendUrl.replace(/\/+$/, "");
  const auth = config.token ? { Authorization: `Bearer ${config.token}` } : {};

  let page = null;
  try {
    page = await browser.tabs.sendMessage(tabId, { type: "hunt-describe" });
  } catch {
    // No content script yet; the fill path injects it. Nothing to describe until then.
  }
  if (!page?.url) return null;

  const res = await fetch(`${base}/hunt/posting-for?url=${encodeURIComponent(page.url)}`, {
    credentials: "include",
    headers: auth,
  });
  const known = res.ok ? await res.json() : null;

  return { page, known, base, auth };
}

async function offerTracking(tabId) {
  const found = await identifyPage(tabId);
  if (!found) return;

  const { page, known } = found;
  if (known?.already_tracked) {
    trackResultEl.textContent = "Already in your tracker.";
    return;
  }
  // Only offer when we can name a posting we collected. Without one there is nothing to
  // snapshot from, and a tracker row of guesses is worse than no row — see the migration's
  // note on internship_applications being renderable from itself alone.
  if (!known?.posting_id) {
    trackResultEl.textContent = "Not one of your collected postings — add it from the site.";
    return;
  }

  trackButton.hidden = false;
  trackButton.textContent = `Track: ${known.company_name || page.company || "this role"}`;
  trackButton.dataset.postingId = known.posting_id;
}

trackButton?.addEventListener("click", async () => {
  const postingId = trackButton.dataset.postingId;
  if (!postingId) return;

  trackResultEl.textContent = "Saving…";
  const config = await settings();
  const base = config.backendUrl.replace(/\/+$/, "");
  try {
    const res = await fetch(`${base}/internships/applications`, {
      method: "POST",
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        ...(config.token ? { Authorization: `Bearer ${config.token}` } : {}),
      },
      body: JSON.stringify({ posting_id: postingId }),
    });
    if (res.status === 409) {
      trackResultEl.textContent = "Already in your tracker.";
    } else if (!res.ok) {
      trackResultEl.textContent = `Could not track it (${res.status}).`;
      return;
    } else {
      trackResultEl.textContent = "Added to your tracker as applied.";
    }
    trackButton.hidden = true;
  } catch (err) {
    trackResultEl.textContent = `Could not track it: ${err.message}`;
  }
});

// Offered on open, so it is available whether or not you used the fill button — you may well
// have typed the form yourself.
void (async () => {
  const tab = await activeTab();
  if (tab) await offerTracking(tab.id).catch(() => {});
})();
