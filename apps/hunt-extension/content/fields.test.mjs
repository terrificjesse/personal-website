/*
 * Loads the shipped fields.js and reads the same global the content script reads, so these
 * tests exercise exactly what runs on a page — not a copy that can drift from it.
 */
import fs from "node:fs";
import vm from "node:vm";

const source = fs.readFileSync(new URL("./fields.js", import.meta.url), "utf8");
const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(source, sandbox);
const { classify, normalizeLabel, inputIsBlocked } = sandbox.HuntFields;

let fail = 0;
const eq = (name, actual, expected) => {
  const ok = JSON.stringify(actual) === JSON.stringify(expected);
  if (!ok) { fail++; console.log(`  FAIL  ${name}\n        got ${JSON.stringify(actual)} want ${JSON.stringify(expected)}`); }
  else console.log(`  pass  ${name}`);
};
const field = (k) => ({ kind: "field", key: k });
const blocked = (r) => ({ kind: "blocked", reason: r });
const skip = { kind: "skip" };

console.log("\n-- the blocklist, which must win over everything --");
for (const label of ["Password", "Confirm Password", "Social Security Number", "SSN",
                     "Credit Card Number", "CVV", "Passport Number", "Driver's License",
                     "Bank Account Number", "Routing number"])
  eq(`"${label}" is blocked`, classify(label), blocked("sensitive"));

console.log("\n-- demographics: never matched, never filled --");
for (const label of ["Race", "Ethnicity", "Gender", "Veteran Status",
                     "Disability Status", "Sexual Orientation", "Date of Birth"])
  eq(`"${label}" is blocked`, classify(label), blocked("demographic"));

console.log("\n-- the ordinary fields --");
eq("First Name", classify("First Name"), field("first_name"));
eq("Last Name *", classify("Last Name *"), field("last_name"));
eq("Email (required)", classify("Email (required)"), field("email"));
eq("Phone Number", classify("Phone Number"), field("phone"));
eq("LinkedIn URL", classify("LinkedIn URL"), field("linkedin_url"));
eq("GitHub Profile", classify("GitHub Profile"), field("github_url"));
eq("School", classify("School"), field("school"));
eq("Expected Graduation Year", classify("Expected Graduation Year"), field("graduation_year"));
eq("What is your GPA?", classify("What is your GPA?"), field("gpa"));
eq("Will you require sponsorship", classify("Will you require visa sponsorship?"), field("needs_sponsorship"));

console.log("\n-- specificity: the longer phrase wins --");
eq('"First name" is not "name"', classify("First name"), field("first_name"));
eq("Preferred First Name -> preferred", classify("Preferred First Name"), field("preferred_name"));
eq("LinkedIn URL beats bare linkedin", classify("LinkedIn URL"), field("linkedin_url"));

console.log("\n-- things that must NOT match --");
for (const label of ["Company Name", "Referrer Name", "Software Engineer",
                     "How did you hear about us?", "Cover Letter", "Why do you want to work here?",
                     "Manager name", ""])
  eq(`"${label}" is skipped`, classify(label), skip);

console.log("\n-- an unmatched field is left alone rather than guessed --");
eq("gibberish", classify("Zorble quux"), skip);

console.log("\n-- input attributes, independent of the label --");
eq("type=password", inputIsBlocked({ type: "password" }), true);
eq("type=file", inputIsBlocked({ type: "file" }), true);
// This test asserted `false` and passed, which is how the gap was found: cc-number is the
// standard autocomplete token for a card number and must be refused.
eq("autocomplete=cc-number", inputIsBlocked({ type: "text", autocomplete: "cc-number" }), true);
eq("autocomplete=cc-csc", inputIsBlocked({ type: "text", autocomplete: "cc-csc" }), true);
eq("autocomplete=email is fine", inputIsBlocked({ type: "text", autocomplete: "email" }), false);
eq('"Drivers License" (no apostrophe)', classify("Drivers License"), blocked("sensitive"));
eq('"Driving Licence" (en-GB)', classify("Driving Licence"), blocked("sensitive"));
eq('"Driver Licence"', classify("Driver Licence"), blocked("sensitive"));
eq("name=creditCardNumber", inputIsBlocked({ type: "text", name: "creditCardNumber" }), true);
eq("plain text input", inputIsBlocked({ type: "text", name: "first_name" }), false);

console.log("\n-- normalization --");
eq("strips markers", normalizeLabel("  First   Name * (required) "), "first name");

console.log("\n-- labels taken verbatim from a live Greenhouse form (Jump Trading) --");
eq("First Name*", classify("First Name*"), field("first_name"));
eq("Location (City)*", classify("Location (City)*"), field("location"));
eq("LinkedIn Profile", classify("LinkedIn Profile"), field("linkedin_url"));
eq("expected graduation date", classify("What is your expected graduation date?*"), field("graduation_date"));
eq("current school, long question", classify("Please select your current school from the list below:*"), field("school"));
eq("degree, as a question", classify("What degree are you currently pursuing?*"), field("degree"));
eq("sponsorship, as a question", classify("Will you require sponsorship for work authorization in "), field("needs_sponsorship"));
// Things on that same form we must NOT touch.
eq("Country* (not stored)", classify("Country*"), skip);
eq("Non-compete comments", classify("Non-compete/Notice period comments*"), skip);
eq("Acknowledge/Confirm", classify("Acknowledge/Confirm"), skip);


console.log("\n-- labels taken verbatim from a live Lever form (Energy Vault) --");
// Lever renders a select's options into the label with no separator. These read as
// "genderselect" / "raceselect" once normalized, and the blocklist silently missed them.
eq("GenderSelect...", classify("GenderSelect ...MaleFemaleDecline to self-identify"), blocked("demographic"));
eq("RaceSelect...", classify("RaceSelect ...Hispanic or LatinoWhite (Not Hispanic or Latino)"), blocked("demographic"));
eq("Veteran statusSelect...", classify("Veteran statusSelect ...I am a veteranI am not a veteran"), blocked("demographic"));
eq("Full name\u2731", classify("Full name\u2731"), field("full_name"));
eq("Current location + noise", classify("Current location No location found. Try entering a different"), field("location"));
eq("GitHub URL", classify("GitHub URL"), field("github_url"));
eq("Portfolio URL", classify("Portfolio URL"), field("portfolio_url"));
// Beside "Portfolio URL" on the same form — filling both put one URL in two questions.
eq("Other website is not the portfolio", classify("Other website"), skip);
eq("Current company (not stored)", classify("Current company \u2731"), skip);
eq("Twitter URL (not stored)", classify("Twitter URL"), skip);
// The unglue step must not cost the matches it could break.
eq("LinkedIn still matches", classify("LinkedIn URL"), field("linkedin_url"));
eq("GitHub still matches", classify("GitHub Profile"), field("github_url"));


console.log("\n-- labels taken verbatim from a live Ashby form (AfterQuery) --");
// Ashby labels the applicant name field simply "Name". Exact-only, so it does not reopen
// the "Company Name" hole that keeping bare "name" out of the synonyms was closing.
eq("bare Name", classify("Name"), field("full_name"));
eq("Company Name still skipped", classify("Company Name"), skip);
eq("Referrer Name still skipped", classify("Referrer Name"), skip);
eq("Ashby Email", classify("Email"), field("email"));
eq("Ashby LinkedIn Profile", classify("LinkedIn Profile"), field("linkedin_url"));
// Rule 11: never touch a CAPTCHA. Ashby puts a real textarea in the form for it.
eq("recaptcha textarea", classify("g-recaptcha-response g-recaptcha-response-100000"), blocked("sensitive"));
eq("captcha, any spelling", classify("Captcha"), blocked("sensitive"));

console.log(fail === 0 ? "\n  ALL PASSED" : `\n  ${fail} FAILED`);
process.exit(fail ? 1 : 0);
