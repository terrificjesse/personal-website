/**
 * Pure helpers for the CV profile editor, kept out of the component so they can be exercised
 * without React.
 *
 * # Normalize on save, never on keystroke
 *
 * The rule is that a cleared input is absent (`null`), not blank (`""`) — because the autofill
 * types whatever it is given, and a blank in a required field looks filled to you and empty to
 * the recruiter.
 *
 * Applying that rule in `onChange` is a bug that reads as correct, and it shipped: trimming
 * every keystroke strips the space the moment you type it, so the next letter lands against
 * the previous word and "Ada Lovelace" is untypeable. The component therefore holds the raw
 * string while you type, and [`forSaving`] normalizes once, at the boundary. The backend
 * enforces the same rule again on write, so neither side depends on the other getting it right.
 */

import type { CvProfile } from "./internshipsApi";

export type CvTextField = { key: keyof CvProfile; label: string; hint?: string };

/** The free-text fields, in the order the editor renders them. */
export const CV_TEXT_FIELDS: CvTextField[] = [
  { key: "full_name", label: "Full name" },
  { key: "first_name", label: "First name", hint: "Some forms ask separately" },
  { key: "last_name", label: "Last name" },
  { key: "preferred_name", label: "Preferred name" },
  { key: "email", label: "Email" },
  { key: "phone", label: "Phone" },
  { key: "location", label: "Location", hint: "As a form asks for it, e.g. “Boston, MA”" },
  { key: "school", label: "School" },
  { key: "degree", label: "Degree" },
  { key: "major", label: "Major" },
  { key: "gpa", label: "GPA" },
  { key: "github_url", label: "GitHub" },
  { key: "linkedin_url", label: "LinkedIn" },
  { key: "portfolio_url", label: "Portfolio" },
  {
    key: "work_authorization",
    label: "Work authorization",
    hint: "Free text — e.g. “US citizen”",
  },
  {
    key: "resume_path",
    label: "Résumé path",
    hint: "A reminder only. Files are never uploaded for you.",
  },
];

/** Trim, and treat whitespace-only as absent. Call at save time, not per keystroke. */
export function cvText(value: string | null): string | null {
  const trimmed = (value ?? "").trim();
  return trimmed === "" ? null : trimmed;
}

/** Parse a numeric field; blank and unparseable both mean absent. */
export function cvNumber(value: string): number | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Normalize every free-text field for the wire. */
export function forSaving(profile: CvProfile): CvProfile {
  const cleaned = { ...profile };
  for (const field of CV_TEXT_FIELDS) {
    (cleaned[field.key] as string | null) = cvText(cleaned[field.key] as string | null);
  }
  return cleaned;
}

/**
 * How many fields carry real content.
 *
 * Not merely "non-null": a field being edited holds `""` between the first keystroke and the
 * save, and an empty box is not a filled one.
 */
export function filledCount(profile: CvProfile): number {
  return Object.values(profile).filter(
    (value) => value !== null && String(value).trim() !== "",
  ).length;
}
