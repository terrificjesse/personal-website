"use client";

/**
 * The CV details the Firefox extension autofills into ATS forms (Phase 8f).
 *
 * # Empty means empty
 *
 * A cleared input is sent as `null`, never `""`. The backend enforces the same rule, but doing
 * it here too means the round trip never briefly carries a blank that the extension could
 * cache — and the extension types whatever it is given. A blank in a required field looks
 * filled to you and empty to the recruiter.
 *
 * # There are no demographic fields here, deliberately
 *
 * Race, gender, veteran and disability questions are opt-in and default off (rule 10). The
 * strongest form of "off" is having nothing to fill: data that is not stored cannot be typed
 * into a form by a bad label match or a future refactor that forgets a flag.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import {
  EMPTY_CV_PROFILE,
  getCvProfile,
  saveCvProfile,
  type CvProfile,
} from "@/lib/internshipsApi";

/** A cleared input is absent, not blank. The one place that conversion happens. */
function text(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

function num(value: string): number | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

const TEXT_FIELDS: { key: keyof CvProfile; label: string; hint?: string }[] = [
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
  { key: "work_authorization", label: "Work authorization", hint: "Free text — e.g. “US citizen”" },
  { key: "resume_path", label: "Résumé path", hint: "A reminder only. Files are never uploaded for you." },
];

export function CvProfileEditor() {
  const handleError = useApiError();
  const [profile, setProfile] = useState<CvProfile>(EMPTY_CV_PROFILE);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await getCvProfile();
        if (!cancelled) setProfile(loaded);
      } catch (err) {
        if (!cancelled) setStatus(handleError(err, "Could not load your CV profile"));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleError]);

  const update = useCallback(
    <K extends keyof CvProfile>(key: K, value: CvProfile[K]) =>
      setProfile((current) => ({ ...current, [key]: value })),
    [],
  );

  async function save() {
    setBusy(true);
    setStatus(null);
    try {
      setProfile(await saveCvProfile(profile));
      setStatus("Saved.");
    } catch (err) {
      setStatus(handleError(err, "Could not save your CV profile"));
    } finally {
      setBusy(false);
    }
  }

  const filled = Object.values(profile).filter((value) => value !== null).length;

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <button
        type="button"
        className="flex w-full items-baseline justify-between gap-2 text-left"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
      >
        <span className="font-semibold">CV details for autofill</span>
        <span className="text-sm text-neutral-500">
          {filled} of {Object.keys(EMPTY_CV_PROFILE).length} filled — {open ? "hide" : "edit"}
        </span>
      </button>

      {open && (
        <>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
            What the extension types into application forms. Blank fields are skipped, never
            filled with an empty value.
          </p>

          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            {TEXT_FIELDS.map((field) => (
              <label key={field.key} className="block text-sm">
                {field.label}
                <input
                  className="mt-1 w-full rounded border border-neutral-300 px-2 py-1 dark:border-neutral-700 dark:bg-neutral-900"
                  value={(profile[field.key] as string | null) ?? ""}
                  maxLength={500}
                  onChange={(event) =>
                    update(field.key, text(event.target.value) as CvProfile[typeof field.key])
                  }
                />
                {field.hint && (
                  <span className="mt-0.5 block text-xs text-neutral-500">{field.hint}</span>
                )}
              </label>
            ))}

            <label className="block text-sm">
              Graduation month
              <input
                type="number"
                min={1}
                max={12}
                className="mt-1 w-full rounded border border-neutral-300 px-2 py-1 dark:border-neutral-700 dark:bg-neutral-900"
                value={profile.graduation_month ?? ""}
                onChange={(event) => update("graduation_month", num(event.target.value))}
              />
            </label>

            <label className="block text-sm">
              Graduation year
              <input
                type="number"
                min={1900}
                max={2100}
                className="mt-1 w-full rounded border border-neutral-300 px-2 py-1 dark:border-neutral-700 dark:bg-neutral-900"
                value={profile.graduation_year ?? ""}
                onChange={(event) => update("graduation_year", num(event.target.value))}
              />
            </label>

            <label className="block text-sm sm:col-span-2">
              Needs sponsorship
              <select
                className="mt-1 w-full rounded border border-neutral-300 px-2 py-1 dark:border-neutral-700 dark:bg-neutral-900"
                value={
                  profile.needs_sponsorship === null
                    ? ""
                    : profile.needs_sponsorship
                      ? "yes"
                      : "no"
                }
                onChange={(event) =>
                  update(
                    "needs_sponsorship",
                    event.target.value === "" ? null : event.target.value === "yes",
                  )
                }
              >
                {/* Three options, not two. "Not stated" is a real answer and the default. */}
                <option value="">Not stated</option>
                <option value="no">No</option>
                <option value="yes">Yes</option>
              </select>
            </label>
          </div>

          <div className="mt-3 flex items-center gap-3">
            <button
              type="button"
              className="rounded border border-neutral-300 px-3 py-1 text-sm disabled:opacity-50 dark:border-neutral-700"
              onClick={save}
              disabled={busy}
            >
              Save
            </button>
            {status && <span className="text-sm text-neutral-600">{status}</span>}
          </div>
        </>
      )}
    </section>
  );
}
