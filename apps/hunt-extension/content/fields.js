/*
 * What a form field is, and whether we may write to it. Pure functions, no DOM.
 *
 * Split out from the filling itself so this can be run and argued with directly — it is the
 * part that decides whether your phone number goes in the phone box or somewhere it must never
 * go, and "it looked right in review" is not the standard for that.
 *
 * # Order is the safety property
 *
 * `classify` checks the blocklist BEFORE it tries to match anything (rule 10: "checked before
 * the fuzzy mapper runs, not after"). A blocked label short-circuits, so no amount of fuzzy
 * matching downstream can talk its way into a password or a card number. Getting this backwards
 * would still pass every happy-path test.
 *
 * # Labels, not selectors
 *
 * ATS markup is regenerated on every redesign; the visible label is what survives. This file
 * only ever sees label text.
 */

/**
 * Never write to a field whose label matches these, whatever else it looks like.
 *
 * The list is about REFUSAL, not about what we store. We hold no card number for a mis-match
 * to leak — but a fuzzy match that decided "Card number" meant "Phone number" would type your
 * phone into a payment field, and that is the failure this prevents.
 */
/*
 * NOTE: every pattern here is matched against `normalizeLabel` output, which has already
 * lowercased and stripped punctuation. So "Driver's License" arrives as "driver s license" —
 * write for that, not for the raw text. A pattern written for the raw form silently matches
 * nothing, which is the failure direction that lets a blocked field through.
 */
const BLOCKED = [
  /password/,
  /\bpin\b/,
  /social security/,
  /\bssn\b/,
  /\bsin\b/,
  /tax\s*(id|identification)/,
  /\bitin\b/,
  /credit\s*card/,
  /debit\s*card/,
  /card\s*number/,
  /\bcvv\b/,
  /\bcvc\b/,
  /security\s*code/,
  /expiry|expiration\s*date/,
  /bank\s*account/,
  /routing\s*number/,
  /\biban\b/,
  /sort\s*code/,
  /passport/,
  // "driver's licence", "drivers license", "driving licence" — all arrive here with the
  // apostrophe already stripped, and all are government ID.
  /driv(er|ing)\s*s?\s*licen[sc]e/,
  /national\s*(id|insurance)/,
  /government\s*id/,
  // The browser's own autocomplete tokens, which arrive normalized as "cc number", "cc csc"
  // and so on. More reliable than any wording, and missed by the prose patterns above.
  /\bcc\s*(number|csc|cvc|exp|expiry|name)\b/,
];

/**
 * Demographic and EEO questions. Never filled, and never even matched.
 *
 * Rule 10 makes these opt-in and default off. `cv_profile` stores nothing of the sort, so
 * there is nothing to type — but they are listed explicitly anyway, because "we happen to hold
 * no value for it" is a weaker guarantee than "we refuse to touch it", and the first quietly
 * stops being true the day somebody adds a field.
 */
const DEMOGRAPHIC = [
  /\brace\b/,
  /ethnicity|ethnic\s*group/,
  /\bgender\b/,
  /\bsex\b/,
  /veteran/,
  /disabilit(y|ies)/,
  /sexual\s*orientation/,
  /transgender/,
  /\blgbt/,
  /date\s*of\s*birth|\bdob\b/,
];

/**
 * Label text that identifies each profile field, most specific first.
 *
 * Deliberately not including bare "name": on a real form it is as likely to be "Company name"
 * or "Referrer name", and a wrong match here writes your legal name into someone else's box.
 * An unmatched field is left alone, which is always the safe outcome.
 */
const SYNONYMS = {
  first_name: ["first name", "given name", "forename", "first"],
  last_name: ["last name", "surname", "family name", "last"],
  preferred_name: ["preferred name", "nickname", "goes by", "preferred first name"],
  full_name: ["full name", "legal name", "your name", "candidate name", "full legal name"],
  email: ["email", "e mail", "email address", "work email", "personal email"],
  phone: ["phone", "phone number", "telephone", "mobile", "mobile number", "cell", "cell phone"],
  location: ["location", "city", "current location", "address", "city and state", "where are you located", "current city"],
  school: ["school", "university", "college", "institution", "school name"],
  degree: ["degree", "degree type", "level of education"],
  major: ["major", "field of study", "discipline", "course of study", "concentration"],
  gpa: ["gpa", "grade point average"],
  graduation_year: ["graduation year", "grad year", "expected graduation year", "year of graduation", "anticipated graduation year"],
  graduation_month: ["graduation month", "grad month", "expected graduation month"],
  github_url: ["github", "github url", "github profile", "github username"],
  linkedin_url: ["linkedin", "linkedin url", "linkedin profile"],
  portfolio_url: ["portfolio", "website", "personal website", "personal site", "portfolio url", "other website"],
  work_authorization: ["work authorization", "authorized to work", "work status", "visa status", "employment authorization"],
  needs_sponsorship: ["sponsorship", "require sponsorship", "need sponsorship", "will you require sponsorship", "visa sponsorship"],
};

/** Lowercase, strip punctuation and required-field markers, collapse whitespace. */
export function normalizeLabel(raw) {
  return (raw || "")
    .toLowerCase()
    .replace(/\*/g, " ")
    .replace(/\(required\)|\(optional\)/g, " ")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

/** Whether `needle` appears in `text` as whole words, so "last" does not match "lastly". */
function containsPhrase(text, needle) {
  return new RegExp(`(^| )${needle.replace(/ /g, "\\s+")}( |$)`).test(text);
}

/**
 * What to do with a field carrying this label.
 *
 * Returns one of:
 *   { kind: "blocked",  reason }  — never write here
 *   { kind: "skip" }              — nothing of ours belongs here
 *   { kind: "field", key }        — fill from `cv_profile[key]`
 */
export function classify(rawLabel) {
  const label = normalizeLabel(rawLabel);
  if (!label) return { kind: "skip" };

  // FIRST. See the header — reordering this is a silent safety regression.
  for (const pattern of BLOCKED) {
    if (pattern.test(label)) return { kind: "blocked", reason: "sensitive" };
  }
  for (const pattern of DEMOGRAPHIC) {
    if (pattern.test(label)) return { kind: "blocked", reason: "demographic" };
  }

  // Exact match wins outright.
  for (const [key, phrases] of Object.entries(SYNONYMS)) {
    if (phrases.includes(label)) return { kind: "field", key };
  }

  // Then whole-phrase containment, collecting every candidate rather than taking the first.
  const matches = new Map();
  for (const [key, phrases] of Object.entries(SYNONYMS)) {
    for (const phrase of phrases) {
      if (containsPhrase(label, phrase)) {
        const best = matches.get(key) || 0;
        matches.set(key, Math.max(best, phrase.length));
      }
    }
  }
  if (matches.size === 0) return { kind: "skip" };

  // One winner, by longest matched phrase — "first name" beats "first", and "linkedin url"
  // beats a bare "linkedin". A genuine tie between DIFFERENT fields is ambiguous, and an
  // ambiguous label is one we leave alone: guessing writes real data into the wrong box, and
  // the cost of skipping is that you type one field yourself.
  const ranked = [...matches.entries()].sort((a, b) => b[1] - a[1]);
  if (ranked.length > 1 && ranked[0][1] === ranked[1][1]) {
    return { kind: "skip" };
  }
  return { kind: "field", key: ranked[0][0] };
}

/**
 * Whether an input is one we are willing to type into at all, on its own attributes.
 *
 * Independent of the label, and checked as well as it — a field can be `type="password"` under
 * a label that says nothing suspicious, and the browser's own `autocomplete` hints are more
 * reliable than any wording.
 */
export function inputIsBlocked({ type, autocomplete, name, id }) {
  const kind = (type || "").toLowerCase();
  if (kind === "password" || kind === "hidden" || kind === "file") return true;

  const hint = normalizeLabel(`${autocomplete || ""} ${name || ""} ${id || ""}`);
  if (!hint) return false;
  return BLOCKED.some((pattern) => pattern.test(hint));
}
