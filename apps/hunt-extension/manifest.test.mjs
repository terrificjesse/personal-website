/*
 * The manifest is a safety surface, so it gets a test like any other.
 *
 * Rule 10 says the content script may load on known ATS hosts and reach everything else only
 * through `activeTab` behind a click, and that `<all_urls>` must never ship. Those are
 * properties of this file, and nothing else in the extension checks them.
 *
 *   node manifest.test.mjs
 */
import fs from "node:fs";

const manifest = JSON.parse(
  fs.readFileSync(new URL("./manifest.json", import.meta.url), "utf8"),
);

let fail = 0;
const ok = (name, condition, detail = "") => {
  if (condition) console.log(`  pass  ${name}`);
  else { fail++; console.log(`  FAIL  ${name}${detail ? `\n        ${detail}` : ""}`); }
};

console.log("\n-- rule 10: never <all_urls>, anywhere --");
const everywhere = JSON.stringify(manifest);
ok("no <all_urls> in the manifest at all", !everywhere.includes("<all_urls>"));

console.log("\n-- the content script's reach is unchanged --");
// Pinned literally. Widening this is how autofill quietly starts running on pages nobody
// audited, and it is a one-line change that looks like a config tweak in a diff.
const expected = [
  "https://boards.greenhouse.io/*",
  "https://job-boards.greenhouse.io/*",
  "https://job-boards.eu.greenhouse.io/*",
  "https://jobs.lever.co/*",
  "https://jobs.ashbyhq.com/*",
  "https://jobs.smartrecruiters.com/*",
];
const matches = manifest.content_scripts.flatMap((entry) => entry.matches);
ok(
  "declarative injection is exactly the six known ATS hosts",
  JSON.stringify(matches) === JSON.stringify(expected),
  `got ${JSON.stringify(matches)}`,
);

console.log("\n-- what may be reached, and when --");
ok(
  "host_permissions granted at install are loopback only",
  manifest.host_permissions.every((pattern) =>
    /^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?\//.test(pattern),
  ),
  `got ${JSON.stringify(manifest.host_permissions)}`,
);
ok(
  "optional host permissions are https-only",
  (manifest.optional_host_permissions ?? []).every((pattern) => pattern.startsWith("https://")),
  `got ${JSON.stringify(manifest.optional_host_permissions)}`,
);
ok(
  "no optional http:// wildcard — a token must not travel in the clear",
  !(manifest.optional_host_permissions ?? []).some((pattern) => pattern.startsWith("http://")),
);

console.log("\n-- permissions stay the ones that were reasoned about --");
ok(
  "permission set unchanged",
  JSON.stringify(manifest.permissions) ===
    JSON.stringify(["alarms", "notifications", "storage", "activeTab", "scripting"]),
  `got ${JSON.stringify(manifest.permissions)}`,
);
ok("the gecko id is still set (settings do not survive a sideload without it)",
   manifest.browser_specific_settings?.gecko?.id === "hunt@personal-website");

console.log(fail === 0 ? "\n  ALL PASSED" : `\n  ${fail} FAILED`);
process.exit(fail ? 1 : 0);
