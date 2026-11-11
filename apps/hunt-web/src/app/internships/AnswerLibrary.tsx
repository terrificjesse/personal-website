"use client";

/**
 * Read, edit and delete the answers the extension offers back to you (Phase 8g).
 *
 * # The badge is the feature
 *
 * An answer flagged company-specific is never offered for a different employer — "why do you
 * want to work at X" reads as the same question everywhere, and pasting one company's answer
 * into another's form is a uniquely bad way to lose an application. That decision is the
 * backend's, recomputed on every edit, and nothing here can override it. Showing the flag is
 * how you find out *why* a suggestion did not appear, which is otherwise invisible.
 *
 * # Edits keep the previous version
 *
 * You improve an answer over time and want the current one — but a rewrite you regret should
 * be recoverable, so every edit writes a revision and the history is one click away.
 */

import { useCallback, useEffect, useState } from "react";
import { useApiError } from "@/lib/useApiError";
import {
  answerRevisions,
  deleteAnswer,
  listAnswers,
  updateAnswer,
  type AnswerRevision,
  type ApplicationAnswer,
} from "@/lib/internshipsApi";

function when(value: string | null): string {
  return value ? new Date(value).toLocaleDateString() : "never used";
}

export function AnswerLibrary() {
  const handleError = useApiError();
  const [answers, setAnswers] = useState<ApplicationAnswer[]>([]);
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [history, setHistory] = useState<Record<string, AnswerRevision[]>>({});
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setAnswers(await listAnswers());
    } catch (err) {
      setStatus(handleError(err, "Could not load your answers"));
    }
  }, [handleError]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await listAnswers();
        if (!cancelled) setAnswers(loaded);
      } catch (err) {
        if (!cancelled) setStatus(handleError(err, "Could not load your answers"));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleError]);

  async function save(id: string) {
    setBusy(true);
    setStatus(null);
    try {
      await updateAnswer(id, draft);
      setEditing(null);
      // The flag may have changed — an edit that names a company makes the answer
      // company-specific — so re-read rather than patching the row locally.
      await refresh();
      setStatus("Saved. The previous version is in the history.");
    } catch (err) {
      setStatus(handleError(err, "Could not save the edit"));
    } finally {
      setBusy(false);
    }
  }

  async function remove(id: string) {
    setBusy(true);
    try {
      await deleteAnswer(id);
      await refresh();
      setStatus("Deleted.");
    } catch (err) {
      setStatus(handleError(err, "Could not delete that answer"));
    } finally {
      setBusy(false);
    }
  }

  async function showHistory(id: string) {
    try {
      setHistory((current) => ({ ...current, [id]: [] }));
      const revisions = await answerRevisions(id);
      setHistory((current) => ({ ...current, [id]: revisions }));
    } catch (err) {
      setStatus(handleError(err, "Could not load the history"));
    }
  }

  return (
    <section className="rounded border border-neutral-300 p-4 dark:border-neutral-700">
      <button
        type="button"
        className="flex w-full items-baseline justify-between gap-2 text-left"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
      >
        <span className="font-semibold">Answer library</span>
        <span className="text-sm text-neutral-500">
          {answers.length} saved — {open ? "hide" : "browse"}
        </span>
      </button>

      {open && (
        <>
          <p className="mt-1 text-sm text-neutral-600 dark:text-neutral-400">
            Questions you have answered before. The extension offers these back on a form;
            answers marked <em>company-specific</em> are never offered to a different employer.
          </p>

          {status && <p className="mt-2 text-sm text-neutral-600">{status}</p>}

          {answers.length === 0 ? (
            <p className="mt-3 text-sm text-neutral-500">
              Nothing yet. Fill an application and use <em>Save my answers from this form</em>{" "}
              in the extension.
            </p>
          ) : (
            <ul className="mt-3 space-y-3">
              {answers.map((answer) => (
                <li key={answer.id} className="border-t border-neutral-200 pt-2 dark:border-neutral-800">
                  <div className="text-sm font-medium">{answer.question_text}</div>

                  <div className="mt-0.5 flex flex-wrap gap-2 text-xs text-neutral-500">
                    {answer.is_company_specific && (
                      <span className="rounded bg-amber-100 px-1.5 py-0.5 text-amber-900 dark:bg-amber-950 dark:text-amber-200">
                        company-specific{answer.company_name ? ` — ${answer.company_name}` : ""}
                      </span>
                    )}
                    <span>used {answer.use_count}×</span>
                    <span>last used {when(answer.last_used_at)}</span>
                  </div>

                  {editing === answer.id ? (
                    <div className="mt-2">
                      <textarea
                        className="w-full rounded border border-neutral-300 p-2 text-sm dark:border-neutral-700 dark:bg-neutral-900"
                        rows={6}
                        value={draft}
                        onChange={(event) => setDraft(event.target.value)}
                      />
                      <div className="mt-1 flex gap-2">
                        <button
                          type="button"
                          className="rounded border border-neutral-300 px-2 py-0.5 text-xs disabled:opacity-50 dark:border-neutral-700"
                          onClick={() => save(answer.id)}
                          disabled={busy}
                        >
                          Save
                        </button>
                        <button
                          type="button"
                          className="rounded border border-neutral-300 px-2 py-0.5 text-xs dark:border-neutral-700"
                          onClick={() => setEditing(null)}
                        >
                          Cancel
                        </button>
                      </div>
                    </div>
                  ) : (
                    <p className="mt-1 whitespace-pre-wrap text-sm">{answer.answer_text}</p>
                  )}

                  <div className="mt-1 flex gap-3 text-xs">
                    <button
                      type="button"
                      className="text-neutral-500 underline"
                      onClick={() => {
                        setEditing(answer.id);
                        setDraft(answer.answer_text);
                      }}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className="text-neutral-500 underline"
                      onClick={() => showHistory(answer.id)}
                    >
                      History
                    </button>
                    <button
                      type="button"
                      className="text-neutral-500 underline disabled:opacity-50"
                      onClick={() => remove(answer.id)}
                      disabled={busy}
                    >
                      Delete
                    </button>
                  </div>

                  {history[answer.id] && (
                    <div className="mt-2 rounded bg-neutral-100 p-2 text-xs dark:bg-neutral-900">
                      {history[answer.id].length === 0 ? (
                        <span className="text-neutral-500">No earlier versions.</span>
                      ) : (
                        history[answer.id].map((revision) => (
                          <div key={revision.replaced_at} className="mb-1">
                            <span className="text-neutral-500">
                              replaced {new Date(revision.replaced_at).toLocaleString()}
                            </span>
                            <p className="whitespace-pre-wrap">{revision.answer_text}</p>
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
