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
  // Rule 11: do not touch CAPTCHAs. Ashby renders a `g-recaptcha-response` textarea into the
  // form, which is a real control our candidate scan sees. Nothing maps to it, so nothing
  // would be typed — but "nothing happens to match" is not a policy, and this is.
  /recaptcha/,
  /captcha/,
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
 * Fields that are about **someone else**, and are therefore never ours to fill.
 *
 * `cv_profile` holds one person's details: yours. A label that names a third party — an
 * emergency contact, a reference, a referrer, a previous employer — is asking for somebody
 * else's information, and the synonym match is *correct* about the word and wrong about the
 * person. "Emergency contact phone" really does contain "phone".
 *
 * Measured on a corpus of realistic ATS labels 2026-09-09: this one shape accounted for 8 of
 * 22 false positives, and it is the worst of them. The others put your own data in the wrong
 * box; these put your phone number where a recruiter expects your reference's.
 *
 * A `skip`, not a `blocked`: nothing here is sensitive, we simply have nothing that belongs in
 * it. Widening the refusal lists to catch a matching bug would misreport why the field was
 * left alone.
 */
const THIRD_PARTY = [
  /emergency\s*contact/,
  /next\s*of\s*kin/,
  /\breferences?\b/,
  /\breferrer\b|referred\s*by|who\s*referred/,
  /\bmanager\b|supervisor/,
  // Your employer is not you. Catches "Last employer" and "Most recent employer name", which
  // otherwise match `last_name` and would type your surname into a company box.
  /\bemployer\b/,
  /\bparent\b|guardian/,
  /\bspouse\b/,
];

/**
 * Openers that mean "write me a paragraph", not "here is a box for your X".
 *
 * "Describe a major challenge you overcame" contains "major"; "Tell us about your degree of
 * involvement" contains "degree". Containment is doing what it was told and the label is still
 * an essay prompt.
 *
 * **Deliberately narrow, and that is the second draft.** The first version treated every
 * interrogative as a prompt — what, when, where, is, are, do, did — and broke four cases the
 * suite already pinned, all of them real: "What is your GPA?", "What is your expected
 * graduation date?", "What degree are you currently pursuing?", "Please select your current
 * school from the list below". Forms ask for ordinary values as questions all the time. Only
 * verbs that ask for prose belong here.
 */
const ESSAY_PROMPT = /^(describe|explain|tell|share|list|walk|why)\b/;

/**
 * "How did you hear about us?" and its variants.
 *
 * Near-universal on ATS forms, and its options are rendered into the label, so it arrives as
 * "How did you hear about us? LinkedIn, Indeed, Referral" — which contains "linkedin" as a
 * whole word and matched `linkedin_url`. The autofill would then type the applicant's LinkedIn
 * profile URL into a sourcing dropdown. Never one of our fields, whatever it contains.
 */
const SOURCING_QUESTION = /\b(how|where)\s+did\s+you\s+(hear|find|learn|discover)\b/;

/**
 * Single-word synonyms that only count when the label **ends** with them.
 *
 * `major` is the case: as a field it is terminal — "Major", "Undergraduate major", "What is
 * your major?" — and everywhere else in English it is an adjective sitting in front of a noun,
 * as in "a major challenge" and "your major accomplishment". Requiring it to be label-final
 * keeps every real field and drops the adjective, which no word list could separate.
 */
const TRAILING_ONLY = new Set(["major"]);

/** Keys whose real-world labels are legitimately question-shaped. */
const ANSWERS_A_QUESTION = new Set(["work_authorization", "needs_sponsorship"]);

/** Whether the label reads as a question at all. Used only to raise the bar, never to lower it. */
function looksLikeAQuestion(raw, label) {
  return /\?/.test(raw || "") || /^(what|when|where|which|who|how|do|does|did|are|is|have|has|can|will|would)\b/.test(label);
}

/**
 * Label text that identifies each profile field, most specific first.
 *
 * Deliberately not including bare "name": on a real form it is as likely to be "Company name"
 * or "Referrer name", and a wrong match here writes your legal name into someone else's box.
 * An unmatched field is left alone, which is always the safe outcome.
 */
/**
 * Labels that count **only as an exact match**, never as a containment.
 *
 * Ashby labels its applicant name field simply "Name". Adding "name" to the synonyms below
 * would also make it match inside "Company Name" and "Referrer Name" — typing your legal name
 * into someone else's box, which is exactly why bare "name" was excluded in the first place.
 * Exact-only gets Ashby's field without reopening that: "company name" is not "name".
 */
const EXACT_ONLY = {
  full_name: ["name"],
  // Same hazard as bare "name", found the same way. Each of these is an ordinary English word
  // that real labels contain without being that field:
  //   "Last employer", "Last company"        -> your surname
  //   "First day available"                  -> your forename
  //   "Mobile development experience"        -> your phone number
  // A form that labels the box just "First" or "Mobile" still matches exactly; nothing that
  // merely contains the word does.
  first_name: ["first"],
  last_name: ["last"],
  phone: ["mobile"],
};

const SYNONYMS = {
  first_name: ["first name", "given name", "forename"],
  last_name: ["last name", "surname", "family name"],
  preferred_name: ["preferred name", "nickname", "goes by", "preferred first name"],
  full_name: ["full name", "legal name", "your name", "candidate name", "full legal name"],
  email: ["email", "e mail", "email address", "work email", "personal email"],
  phone: ["phone", "phone number", "telephone", "mobile number", "mobile phone", "cell", "cell phone"],
  location: ["location", "city", "current location", "address", "city and state", "where are you located", "current city"],
  school: ["school", "university", "college", "institution", "school name"],
  degree: ["degree", "degree type", "level of education"],
  major: ["major", "field of study", "discipline", "course of study", "concentration"],
  gpa: ["gpa", "grade point average"],
  graduation_year: ["graduation year", "grad year", "expected graduation year", "year of graduation", "anticipated graduation year"],
  // Real forms ask for a *date* more often than a year — Jump Trading's Greenhouse form says
  // "What is your expected graduation date?". Rendered from month + year; see `renderValue`.
  graduation_date: ["graduation date", "expected graduation date", "anticipated graduation date", "grad date", "expected graduation"],
  graduation_month: ["graduation month", "grad month", "expected graduation month"],
  github_url: ["github", "github url", "github profile", "github username"],
  linkedin_url: ["linkedin", "linkedin url", "linkedin profile"],
  // "other website" is deliberately absent: on Lever it sits beside "Portfolio URL", and
  // treating both as the portfolio typed the same URL into two different questions.
  portfolio_url: ["portfolio", "personal website", "personal site", "portfolio url"],
  work_authorization: ["work authorization", "authorized to work", "work status", "visa status", "employment authorization"],
  needs_sponsorship: ["sponsorship", "require sponsorship", "need sponsorship", "will you require sponsorship", "visa sponsorship"],
};

/**
 * Insert a space at a lower-to-upper case change, so run-together words separate.
 *
 * Lever renders a `<select>`'s option text straight into the label with no separator:
 * "Gender" arrives as `GenderSelect ...MaleFemaleDecline to self-identify`. Normalized that is
 * "genderselect", and `\bgender\b` never fires — the demographic blocklist silently failed on
 * a real form while appearing to work, because "Veteran status" happened to be matched by a
 * pattern without a word boundary.
 *
 * Used for the REFUSAL checks only, never for synonym matching: splitting this way also turns
 * "LinkedIn" into "linked in" and "GitHub" into "git hub", which would break two matches that
 * work today. Checking the blocklist against both forms can only ever refuse more.
 */
function splitRunTogetherWords(text) {
  return (text || "").replace(/([a-z0-9])([A-Z])/g, "$1 $2");
}

/** Lowercase, strip punctuation and required-field markers, collapse whitespace. */
function normalizeLabel(raw) {
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
function classify(rawLabel) {
  const label = normalizeLabel(rawLabel);
  if (!label) return { kind: "skip" };

  // The refusal checks see both the plain form and the run-together-words form, because a
  // label that reads "GenderSelect" in the DOM is still a gender question. Two chances to
  // refuse, one to match.
  const unglued = normalizeLabel(splitRunTogetherWords(rawLabel));
  const refuses = (pattern) => pattern.test(label) || pattern.test(unglued);

  // FIRST. See the header — reordering this is a silent safety regression.
  for (const pattern of BLOCKED) {
    if (refuses(pattern)) return { kind: "blocked", reason: "sensitive" };
  }
  for (const pattern of DEMOGRAPHIC) {
    if (refuses(pattern)) return { kind: "blocked", reason: "demographic" };
  }

  // Then: is this field even about you? A third-party label is skipped before any matching,
  // for the same reason the blocklist is — once "Emergency contact phone" reaches the matcher
  // it will find "phone", correctly, and be wrong about whose.
  for (const pattern of THIRD_PARTY) {
    if (refuses(pattern)) return { kind: "skip" };
  }

  // An essay prompt or a sourcing dropdown is never one of our fields, whatever words it
  // happens to contain. Checked before matching, for the same reason the blocklist is.
  if (ESSAY_PROMPT.test(label) || SOURCING_QUESTION.test(label)) return { kind: "skip" };

  // Exact match wins outright, including the exact-only labels. Checked BEFORE the prompt
  // shape, because an exact label is a field name however it reads — a box labelled exactly
  // "Do you have a LinkedIn" is not a thing, but if it ever is, it means what it says.
  for (const [key, phrases] of Object.entries(SYNONYMS)) {
    if (phrases.includes(label)) return { kind: "field", key };
  }
  for (const [key, phrases] of Object.entries(EXACT_ONLY)) {
    if (phrases.includes(label)) return { kind: "field", key };
  }

  // Then whole-phrase containment, collecting every candidate rather than taking the first.
  const matches = new Map();
  for (const [key, phrases] of Object.entries(SYNONYMS)) {
    for (const phrase of phrases) {
      if (!containsPhrase(label, phrase)) continue;
      // "major" is a field at the end of a label and an adjective anywhere else.
      if (TRAILING_ONLY.has(phrase) && !label.endsWith(phrase)) continue;
      const best = matches.get(key) || 0;
      matches.set(key, Math.max(best, phrase.length));
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

  // A *question* that matches two different fields is asking about one of them and merely
  // mentioning the other: "What city is your school located in?" matches `location` and
  // `school`, and wants neither your city nor your school's name. On a plain label the
  // longest-phrase rule is a fair tie-break; inside a question it is a guess, and guessing
  // writes real data into the wrong box.
  const key = ranked[0][0];
  if (ranked.length > 1 && looksLikeAQuestion(rawLabel, label) && !ANSWERS_A_QUESTION.has(key)) {
    return { kind: "skip" };
  }
  return { kind: "field", key };
}

/**
 * Whether an input is one we are willing to type into at all, on its own attributes.
 *
 * Independent of the label, and checked as well as it — a field can be `type="password"` under
 * a label that says nothing suspicious, and the browser's own `autocomplete` hints are more
 * reliable than any wording.
 */
function inputIsBlocked({ type, autocomplete, name, id }) {
  const kind = (type || "").toLowerCase();
  if (kind === "password" || kind === "hidden" || kind === "file") return true;

  const hint = normalizeLabel(`${autocomplete || ""} ${name || ""} ${id || ""}`);
  if (!hint) return false;
  return BLOCKED.some((pattern) => pattern.test(hint));
}

/*
 * Exposed as a global rather than as an ES module, deliberately.
 *
 * A module would have to be reachable through `web_accessible_resources` so the content script
 * could `import()` it — and for the activeTab path, that means declaring it reachable from
 * every page, which is exposure bought for nothing. Two classic scripts injected together need
 * no such declaration. `content/fields.test.mjs` evaluates this file and reads the same global,
 * so the tests exercise exactly what ships.
 */
globalThis.HuntFields = { normalizeLabel, classify, inputIsBlocked };
