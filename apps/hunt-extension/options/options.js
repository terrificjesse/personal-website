/*
 * Settings, plus the one diagnostic that matters.
 *
 * "Test connection" exists because the three ways this can be broken produce the same
 * symptom — no notifications — and they have completely different fixes:
 *
 *   the origin isn't in host_permissions  -> edit manifest.json and reload the extension
 *   the backend isn't running             -> start it
 *   the backend doesn't know who we are   -> sign in on the site
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
  const permitted = await browser.permissions.contains({ origins: [`${origin}/*`] });
  if (!permitted) {
    return say(
      `This extension has no permission for ${origin}. Add it to host_permissions in ` +
        `manifest.json and reload the extension.`,
      "bad",
    );
  }

  let response;
  try {
    response = await fetch(`${base}/hunt/events?limit=1`, { credentials: "include" });
  } catch (err) {
    return say(`Can't reach ${origin}. Is the backend running? (${err})`, "bad");
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
