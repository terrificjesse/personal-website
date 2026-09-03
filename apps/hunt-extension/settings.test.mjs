/*
 * The alert kinds have to agree across three files that no compiler checks together:
 * background.js decides what may notify, options.js reads and writes the checkboxes, and
 * options.html holds them. A kind added to one and not the others fails silently and in the
 * worst direction — a producer that ships and never notifies, or a checkbox that cannot be
 * switched off because saving drops the key.
 *
 *   node settings.test.mjs
 */
import fs from "node:fs";

const read = (name) => fs.readFileSync(new URL(name, import.meta.url), "utf8");

let fail = 0;
const ok = (name, condition, detail = "") => {
  if (condition) console.log(`  pass  ${name}`);
  else { fail++; console.log(`  FAIL  ${name}${detail ? `\n        ${detail}` : ""}`); }
};

/** The `kinds: { … }` object literal, as a set of keys. */
const kindsIn = (source) => {
  const match = source.match(/kinds:\s*\{([^}]*)\}/);
  if (!match) return null;
  return new Set([...match[1].matchAll(/(\w+)\s*:/g)].map((m) => m[1]).sort());
};

const background = kindsIn(read("./background.js"));
const options = kindsIn(read("./options/options.js"));
const html = read("./options/options.html");
const optionsJs = read("./options/options.js");

console.log("\n-- every kind is known to every file --");
ok("background.js declares the kinds", background !== null && background.size > 0);
ok(
  "options.js declares the same set",
  JSON.stringify([...background]) === JSON.stringify([...options]),
  `background=${[...background]} options=${[...options]}`,
);

for (const kind of background) {
  ok(`${kind}: has a checkbox in options.html`, html.includes(`id="kind-${kind}"`));
  ok(`${kind}: is READ into the form`, optionsJs.includes(`getElementById("kind-${kind}").checked =`));
  // Without this the checkbox renders, cannot be turned off, and looks like it worked.
  ok(`${kind}: is WRITTEN back on save`, new RegExp(`${kind}:\\s*document\\.getElementById\\("kind-${kind}"\\)\\.checked`).test(optionsJs));
}

console.log("\n-- an unknown kind must be enabled, not muted --");
ok(
  "background.js decides it through a named rule, not an inline comparison",
  /function kindEnabled\(settings, kind\)\s*\{\s*return settings\.kinds\[kind\] !== false;/.test(read("./background.js")),
);

console.log(fail === 0 ? "\n  ALL PASSED" : `\n  ${fail} FAILED`);
process.exit(fail ? 1 : 0);
