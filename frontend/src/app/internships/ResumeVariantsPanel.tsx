"use client";

/**
 * Manage the résumé labels used to attribute internship outcomes.
 *
 * A variant names a file maintained outside the app; this panel never uploads or stores the
 * résumé itself. Retiring sets `archived_at` and leaves the row visible because an old résumé
 * is exactly what a replacement should be compared against. Deletion remains available for an
 * unused mistake, but the backend's 409 for an attributed variant is explained as preservation
 * of history rather than shown as a generic failed request. Invalid and duplicate labels have
 * their own messages, and the visible application count makes the id-stable rename guarantee
 * legible before the user edits a name.
 */

import { useEffect, useState, type FormEvent } from "react";
import {
  createResumeVariant,
  deleteResumeVariant,
  listResumeVariants,
  updateResumeVariant,
  type ResumeVariant,
} from "@/lib/internshipsApi";
import { useApiError } from "@/lib/useApiError";

type Notice = { kind: "error" | "success"; text: string };

function ordered(variants: ResumeVariant[]): ResumeVariant[] {
  return [...variants].sort((left, right) => {
    const archiveOrder =
      Number(left.archived_at !== null) - Number(right.archived_at !== null);
    return (
      archiveOrder ||
      left.label.localeCompare(right.label, undefined, { sensitivity: "base" })
    );
  });
}

export function ResumeVariantsPanel() {
  const handleError = useApiError();
  const [variants, setVariants] = useState<ResumeVariant[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [label, setLabel] = useState("");
  const [notes, setNotes] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editNotes, setEditNotes] = useState("");

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await listResumeVariants();
        if (!cancelled) {
          setVariants(loaded);
          setLoadFailed(false);
        }
      } catch (error) {
        if (!cancelled) {
          setLoadFailed(true);
          setNotice({
            kind: "error",
            text: handleError(
              error,
              error instanceof Error ? error.message : "Could not load résumé variants.",
            ),
          });
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleError]);

  async function retryLoad() {
    setLoading(true);
    setLoadFailed(false);
    setNotice(null);
    try {
      setVariants(await listResumeVariants());
    } catch (error) {
      setLoadFailed(true);
      setNotice({
        kind: "error",
        text: handleError(
          error,
          error instanceof Error ? error.message : "Could not load résumé variants.",
        ),
      });
    } finally {
      setLoading(false);
    }
  }

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setBusyAction("create");
    setNotice(null);
    try {
      const created = await createResumeVariant({ label, notes });
      setVariants((current) => ordered([...current, created]));
      setLabel("");
      setNotes("");
      setNotice({ kind: "success", text: "Résumé variant created." });
    } catch (error) {
      setNotice({
        kind: "error",
        text: handleError(
          error,
          error instanceof Error ? error.message : "Could not create the résumé variant.",
        ),
      });
    } finally {
      setBusyAction(null);
    }
  }

  function beginEdit(variant: ResumeVariant) {
    setEditingId(variant.id);
    setEditLabel(variant.label);
    setEditNotes(variant.notes ?? "");
    setNotice(null);
  }

  async function save(id: string) {
    setBusyAction(`save:${id}`);
    setNotice(null);
    try {
      // An explicit empty string clears notes; omitting the field means "leave unchanged".
      const updated = await updateResumeVariant(id, { label: editLabel, notes: editNotes });
      setVariants((current) =>
        ordered(current.map((variant) => (variant.id === id ? updated : variant))),
      );
      setEditingId(null);
      setNotice({ kind: "success", text: "Résumé variant saved." });
    } catch (error) {
      setNotice({
        kind: "error",
        text: handleError(
          error,
          error instanceof Error ? error.message : "Could not save the résumé variant.",
        ),
      });
    } finally {
      setBusyAction(null);
    }
  }

  async function setRetired(variant: ResumeVariant, archived: boolean) {
    setBusyAction(`archive:${variant.id}`);
    setNotice(null);
    try {
      const updated = await updateResumeVariant(variant.id, { archived });
      setVariants((current) =>
        ordered(current.map((item) => (item.id === variant.id ? updated : item))),
      );
      setNotice({
        kind: "success",
        text: archived
          ? `${variant.label} retired. Its historical results remain visible.`
          : `${variant.label} restored for new applications.`,
      });
    } catch (error) {
      setNotice({
        kind: "error",
        text: handleError(
          error,
          error instanceof Error ? error.message : "Could not change the variant status.",
        ),
      });
    } finally {
      setBusyAction(null);
    }
  }

  async function remove(variant: ResumeVariant) {
    if (!window.confirm(`Delete the résumé variant “${variant.label}”?`)) return;

    setBusyAction(`delete:${variant.id}`);
    setNotice(null);
    try {
      await deleteResumeVariant(variant.id);
      setVariants((current) => current.filter((item) => item.id !== variant.id));
      if (editingId === variant.id) setEditingId(null);
      setNotice({ kind: "success", text: "Unused résumé variant deleted." });
    } catch (error) {
      setNotice({
        kind: "error",
        text: handleError(
          error,
          error instanceof Error ? error.message : "Could not delete the résumé variant.",
        ),
      });
    } finally {
      setBusyAction(null);
    }
  }

  const busy = busyAction !== null;

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <h2 className="font-semibold">Résumé variants</h2>
      <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
        Name the résumés you keep outside this app so applications and response rates can be
        attributed honestly. Renaming is safe: applications stay linked to the same variant,
        and the count beside each name shows how much history follows it. Retired variants stay
        here for comparison.
      </p>

      <form
        onSubmit={create}
        className="mt-3 grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)_auto] sm:items-start"
      >
        <label className="text-sm">
          <span className="mb-1 block text-xs text-neutral-500">Name</span>
          <input
            required
            maxLength={120}
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            placeholder="one-page, systems"
            className="w-full rounded border border-neutral-300 bg-transparent px-2 py-1 dark:border-neutral-700"
          />
        </label>
        <label className="text-sm">
          <span className="mb-1 block text-xs text-neutral-500">What changed? (optional)</span>
          <input
            maxLength={2000}
            value={notes}
            onChange={(event) => setNotes(event.target.value)}
            placeholder="More systems projects; shorter coursework section"
            className="w-full rounded border border-neutral-300 bg-transparent px-2 py-1 dark:border-neutral-700"
          />
        </label>
        <button
          type="submit"
          disabled={busy || loading || loadFailed || label.trim().length === 0}
          className="mt-5 rounded border border-neutral-300 px-3 py-1 text-sm disabled:opacity-50 dark:border-neutral-700"
        >
          {busyAction === "create" ? "Creating…" : "Create"}
        </button>
      </form>

      {notice && (
        <p
          role={notice.kind === "error" ? "alert" : "status"}
          className={`mt-3 rounded px-3 py-2 text-sm ${
            notice.kind === "error"
              ? "border border-red-500/40 bg-red-500/5 text-red-700 dark:text-red-400"
              : "bg-neutral-100 text-neutral-700 dark:bg-neutral-900 dark:text-neutral-300"
          }`}
        >
          {notice.text}
        </p>
      )}

      {loading ? (
        <p className="mt-3 text-sm text-neutral-500">Loading résumé variants…</p>
      ) : loadFailed ? (
        <div className="mt-3 flex items-center gap-3 text-sm text-neutral-500">
          <span>The current variant list is unknown.</span>
          <button
            type="button"
            onClick={retryLoad}
            className="underline disabled:opacity-50"
            disabled={loading}
          >
            Retry
          </button>
        </div>
      ) : variants.length === 0 ? (
        <p className="mt-3 text-sm text-neutral-500">
          No variants yet. Create one here and it will appear in the extension when you track
          an application.
        </p>
      ) : (
        <ul className="mt-4 space-y-3">
          {variants.map((variant) => {
            const retired = variant.archived_at !== null;
            const editing = editingId === variant.id;
            return (
              <li
                key={variant.id}
                className="rounded border border-neutral-200 p-3 dark:border-neutral-800"
              >
                {editing ? (
                  <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
                    <label className="text-sm">
                      <span className="mb-1 block text-xs text-neutral-500">Name</span>
                      <input
                        required
                        maxLength={120}
                        value={editLabel}
                        onChange={(event) => setEditLabel(event.target.value)}
                        className="w-full rounded border border-neutral-300 bg-transparent px-2 py-1 dark:border-neutral-700"
                      />
                    </label>
                    <label className="text-sm">
                      <span className="mb-1 block text-xs text-neutral-500">Notes</span>
                      <input
                        maxLength={2000}
                        value={editNotes}
                        onChange={(event) => setEditNotes(event.target.value)}
                        className="w-full rounded border border-neutral-300 bg-transparent px-2 py-1 dark:border-neutral-700"
                      />
                    </label>
                  </div>
                ) : (
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div>
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{variant.label}</span>
                        {retired && (
                          <span className="rounded bg-neutral-200 px-1.5 py-0.5 text-xs text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
                            Retired
                          </span>
                        )}
                      </div>
                      {variant.notes && (
                        <p className="mt-1 whitespace-pre-wrap text-sm text-neutral-600 dark:text-neutral-400">
                          {variant.notes}
                        </p>
                      )}
                    </div>
                    <span className="text-xs tabular-nums text-neutral-500">
                      Used by {variant.application_count} application
                      {variant.application_count === 1 ? "" : "s"}
                    </span>
                  </div>
                )}

                <div className="mt-2 flex flex-wrap gap-3 text-xs">
                  {editing ? (
                    <>
                      <button
                        type="button"
                        onClick={() => save(variant.id)}
                        disabled={busy || editLabel.trim().length === 0}
                        className="text-neutral-600 underline disabled:opacity-50 dark:text-neutral-400"
                      >
                        {busyAction === `save:${variant.id}` ? "Saving…" : "Save"}
                      </button>
                      <button
                        type="button"
                        onClick={() => setEditingId(null)}
                        disabled={busy}
                        className="text-neutral-600 underline disabled:opacity-50 dark:text-neutral-400"
                      >
                        Cancel
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      onClick={() => beginEdit(variant)}
                      disabled={busy}
                      className="text-neutral-600 underline disabled:opacity-50 dark:text-neutral-400"
                    >
                      Rename / edit notes
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => setRetired(variant, !retired)}
                    disabled={busy}
                    className="text-neutral-600 underline disabled:opacity-50 dark:text-neutral-400"
                  >
                    {busyAction === `archive:${variant.id}`
                      ? retired
                        ? "Restoring…"
                        : "Retiring…"
                      : retired
                        ? "Restore"
                        : "Retire"}
                  </button>
                  <button
                    type="button"
                    onClick={() => remove(variant)}
                    disabled={busy}
                    className="text-red-700 underline disabled:opacity-50 dark:text-red-400"
                  >
                    {busyAction === `delete:${variant.id}` ? "Deleting…" : "Delete"}
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
