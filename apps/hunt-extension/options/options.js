/*
 * Settings, plus the one diagnostic that matters.
 *
 * "Test connection" exists because the ways this can be broken produce the same symptom —
 * no notifications — and they have completely different fixes:
 *
 *   the origin isn't granted              -> grant it (this button asks; see below)
 *   the origin isn't in host_permissions  -> edit manifest.json and reload the extension
 *   the backend isn't running             -> start it
 *   the backend doesn't know who we are   -> sign in on the site
 *
 * # Firefox does NOT grant host_permissions just because the manifest asks
 *
 * This cost an evening. In Chrome, `host_permissions` in the manifest are granted at install
 * and `fetch` to that origin just works. **Firefox MV3 treats them as optional** — the user
 * grants them per-origin, and until they do, a fetch fails the same way an unreachable server
 * does: a bare `TypeError`, with nothing to say permission was the problem.
 *
 * So this button asks for the grant rather than assuming it. `permissions.request` must be
 * called from a user gesture, which a click on this button is; calling it from the background
 * page's alarm would be rejected outright, and that is why the ask lives here and not there.
 *
 * The last one is the one to watch. The extension authenticates with the site's own
 * `fridge_session` cookie, which is `SameSite=Lax`, and a request from a `moz-extension://`
 * page to the backend is cross-site. If Firefox declines to attach the cookie, this button is
 * where that shows up — as "signed out" rather than as silence.
 */

const SETTINGS_KEY = "settings";

const DEFAULTS = {
  backendUrl: "http://localhost:8080",
  siteUrl: "http://localhost:3000",
  pollMinutes: 5,
  kinds: { posting: true, email: true },
  maxNotificationsPerPoll: 3,
};

const fields = ["backendUrl", "siteUrl", "pollMinutes", "maxNotificationsPerPoll"];
const resultEl = document.getElementById("result");

function say(text, tone = "") {
  resultEl.textContent = text;
  resultEl.className = `result ${tone}`;
}

async function load() {
  const stored = await browser.storage.local.get(SETTINGS_KEY);
  const settings = { ...DEFAULTS, ...(stored[SETTINGS_KEY] || {}) };
  settings.kinds = { ...DEFAULTS.kinds, ...(settings.kinds || {}) };

  for (const field of fields) {
    document.getElementById(field).value = settings[field];
  }
  document.getElementById("kind-posting").checked = settings.kinds.posting !== false;
  document.getElementById("kind-email").checked = settings.kinds.email !== false;
}

function fromForm() {
  return {
    backendUrl: document.getElementById("backendUrl").value.trim(),
    siteUrl: document.getElementById("siteUrl").value.trim(),
    pollMinutes: Math.max(1, Number(document.getElementById("pollMinutes").value) || 5),
    maxNotificationsPerPoll: Math.max(
      1,
      Number(document.getElementById("maxNotificationsPerPoll").value) || 3,
    ),
    kinds: {
      posting: document.getElementById("kind-posting").checked,
      email: document.getElementById("kind-email").checked,
    },
  };
}

document.getElementById("form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const settings = fromForm();
  await browser.storage.local.set({ [SETTINGS_KEY]: settings });
  // Re-arm the alarm at the new interval and poll straight away, so saving is visibly a
  // change rather than something that takes effect at some unstated later point.
  await browser.runtime.sendMessage({ type: "settings-changed" });
  say("Saved.", "ok");
});

document.getElementById("test").addEventListener("click", async () => {
  const settings = fromForm();
  const base = settings.backendUrl.replace(/\/+$/, "");
  say("Testing…");

  let origin;
  try {
    origin = new URL(base).origin;
  } catch {
    return say(`${settings.backendUrl} is not a valid URL.`, "bad");
  }

  // Asked first, because without it the fetch below fails in a way that looks exactly like
  // the backend being down.
  const wanted = { origins: [`${origin}/*`] };
  let permitted = await browser.permissions.contains(wanted);

  if (!permitted) {
    try {
      // The prompt. Firefox shows it because this is running inside a click handler.
      permitted = await browser.permissions.request(wanted);
    } catch (err) {
      return say(
        `${origin} is not one of this extension's host_permissions, so it cannot be ` +
          `granted at runtime. Add it to manifest.json and reload the extension. (${err})`,
        "bad",
      );
    }
    if (!permitted) {
      return say(
        `Access to ${origin} was declined. The extension cannot poll for alerts without it — ` +
          `press Test connection again to be asked once more.`,
        "bad",
      );
    }
  }

  let response;
  try {
    response = await fetch(`${base}/hunt/events?limit=1`, { credentials: "include" });
  } catch (err) {
    // A credentialed cross-origin response with no `Access-Control-Allow-Origin` is DISCARDED
    // by the browser before this code can read it, and surfaces here as the same bare
    // TypeError a dead server produces. The backend's ALLOWED_ORIGINS has to name this
    // extension, so print the origin it needs to be told about rather than making the user go
    // digging for a per-profile UUID.
    const self = browser.runtime.getURL("").replace(/\/$/, "");
    // Name the permission case explicitly: a blocked request and a dead server both arrive
    // here as a bare TypeError, and telling them apart by hand is what wasted the evening.
    const stillPermitted = await browser.permissions.contains(wanted);
    if (!stillPermitted) {
      return say(`Access to ${origin} is not granted, so the request was blocked.`, "bad");
    }
    return say(
      `${origin} did not answer this extension. If the site itself works in a tab, the ` +
        `backend is running and the response is being discarded for CORS — add this exact ` +
        `origin to ALLOWED_ORIGINS in the backend .env and restart it:\n\n${self}\n\n` +
        `(${err.name}: ${err.message})`,
      "bad",
    );
  }

  if (response.status === 401) {
    return say(
      `${origin} is reachable but doesn't recognise the session. Open ${settings.siteUrl}, ` +
        `sign in, then test again.`,
      "bad",
    );
  }
  if (!response.ok) {
    return say(`${origin} answered HTTP ${response.status}.`, "bad");
  }

  const payload = await response.json();
  say(`Connected and signed in. ${payload.unacked_total} alert(s) waiting to be shown.`, "ok");
});

load();
