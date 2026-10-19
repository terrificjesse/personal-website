"use client";

/**
 * Label held-out email, one message at a time, for measuring the classifier (Phase 13b).
 *
 * Deliberately never shows what the classifier decided — see `inbox::labelling` in the backend,
 * which enforces it. Keyboard-first: 1–7 pick a category, 0 skips, U undoes, because forty of
 * these by mouse is the difference between doing it and not.
 */

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { useApiError } from "@/lib/useApiError";
import {
  LABEL_CHOICES,
  formatDate,
  getLabellingProgress,
  nextToLabel,
  saveLabel,
  undoLabel,
  type LabellingItem,
  type LabellingProgress,
} from "@/lib/internshipsApi";

export default function LabellingPage() {
  const handleApiError = useApiError();
  const [progress, setProgress] = useState<LabellingProgress | null>(null);
  const [item, setItem] = useState<LabellingItem | null>(null);
  const [started, setStarted] = useState(false);
  const [done, setDone] = useState(false);
  const [busy, setBusy] = useState(false);
  const [lastId, setLastId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loaded = await getLabellingProgress();
        if (!cancelled) setProgress(loaded);
      } catch (err) {
        if (!cancelled) setError(handleApiError(err, "Could not load labelling progress"));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [handleApiError]);

  const loadNext = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await nextToLabel();
      setProgress(next.progress);
      setItem(next.item);
      setDone(next.item === null);
      setStarted(true);
    } catch (err) {
      setError(handleApiError(err, "Could not load the next email"));
    } finally {
      setBusy(false);
    }
  }, [handleApiError]);

  const choose = useCallback(
    async (label: string | null) => {
      if (!item || busy) return;
      setBusy(true);
      setError(null);
      try {
        await saveLabel(item.message_id, label);
        setLastId(item.message_id);
      } catch (err) {
        setError(handleApiError(err, "Could not save that label"));
        setBusy(false);
        return;
      }
      setBusy(false);
      await loadNext();
    },
    [item, busy, handleApiError, loadNext],
  );

  const undo = useCallback(async () => {
    if (!lastId || busy) return;
    setBusy(true);
    try {
      await undoLabel(lastId);
      setLastId(null);
    } catch (err) {
      setError(handleApiError(err, "Could not undo that label"));
      setBusy(false);
      return;
    }
    setBusy(false);
    // The shuffle is fixed and labelling follows it, so the undone message is the earliest
    // unlabelled one again and comes straight back.
    await loadNext();
  }, [lastId, busy, handleApiError, loadNext]);

  useEffect(() => {
    if (!started || done) return;
    function onKey(event: KeyboardEvent) {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const index = Number(event.key) - 1;
      if (index >= 0 && index < LABEL_CHOICES.length) {
        event.preventDefault();
        void choose(LABEL_CHOICES[index].value);
      } else if (event.key === "0" || event.key.toLowerCase() === "s") {
        event.preventDefault();
        void choose(null);
      } else if (event.key.toLowerCase() === "u") {
        event.preventDefault();
        void undo();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [started, done, choose, undo]);

  const reachedTarget = progress !== null && progress.labelled >= progress.target;

  return (
    <main className="mx-auto max-w-3xl px-4 py-8">
      <Link href="/internships/applications" className="text-sm underline">
        ← Applications
      </Link>
      <h1 className="mt-2 text-2xl font-semibold">Label your email</h1>
      <p className="mt-2 text-sm text-neutral-600 dark:text-neutral-400">
        This measures how accurately your email is sorted. Read each message and pick what it
        actually is. You won&apos;t see what the sorter decided — that&apos;s on purpose, so it
        can&apos;t sway you. Forty is enough for a first measurement, and you can stop at any
        point: the order is shuffled, so whatever you&apos;ve done is a fair sample.
      </p>

      {progress && (
        <div className="mt-4">
          <div className="flex flex-wrap gap-x-4 text-sm tabular-nums text-neutral-600 dark:text-neutral-400">
            <span>
              <strong className="text-neutral-900 dark:text-neutral-100">{progress.labelled}</strong>{" "}
              labelled
            </span>
            <span>{progress.skipped} skipped</span>
            <span>{progress.remaining} left</span>
            <span className="ml-auto">target {progress.target}</span>
          </div>
          <div
            className="mt-1 h-1.5 overflow-hidden rounded bg-neutral-200 dark:bg-neutral-800"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={progress.target}
            aria-valuenow={Math.min(progress.labelled, progress.target)}
          >
            <div
              className="h-full bg-sky-600"
              style={{ width: `${Math.min(100, (progress.labelled / progress.target) * 100)}%` }}
            />
          </div>
          {reachedTarget && (
            <p className="mt-2 text-sm text-green-700 dark:text-green-400">
              That&apos;s enough for a first measurement. Keep going if you like, or stop here and
              ask for it to be scored.
            </p>
          )}
        </div>
      )}

      {error && <p className="mt-4 text-sm text-red-700 dark:text-red-400">{error}</p>}

      {!started && (
        <button
          type="button"
          className="mt-6 rounded border border-black/20 px-3 py-1.5 text-sm disabled:opacity-50 dark:border-white/25"
          onClick={() => void loadNext()}
          disabled={busy || progress === null || progress.remaining === 0}
        >
          {progress && progress.labelled + progress.skipped > 0 ? "Continue" : "Start"}
        </button>
      )}

      {started && done && (
        <p className="mt-6 text-sm">
          Every held-out email is labelled. Ask for it to be scored — the labels are saved.
        </p>
      )}

      {started && item && (
        <article className="mt-6 rounded-lg border border-black/10 p-4 dark:border-white/15">
          <header className="text-sm">
            <div className="font-medium">{item.subject || "(no subject)"}</div>
            <div className="text-neutral-600 dark:text-neutral-400">{item.from}</div>
            {item.received_at && (
              <div className="text-xs text-neutral-500">{formatDate(item.received_at)}</div>
            )}
          </header>

          {item.body ? (
            <div className="mt-3 max-h-96 overflow-y-auto whitespace-pre-wrap break-words text-sm leading-relaxed">
              {item.body}
            </div>
          ) : (
            <>
              <p className="mt-3 text-xs text-amber-700 dark:text-amber-400">
                Couldn&apos;t load the full email, so this is only the preview. If the preview
                doesn&apos;t settle it, open the email in Gmail — a polite rejection often only
                says so further down.
              </p>
              <div className="mt-2 whitespace-pre-wrap break-words text-sm">{item.snippet}</div>
            </>
          )}

          {item.gmail_url && (
            <a
              className="mt-2 inline-block text-xs underline"
              href={item.gmail_url}
              target="_blank"
              rel="noreferrer"
            >
              Open in Gmail
            </a>
          )}

          <div className="mt-4 grid gap-2 sm:grid-cols-2">
            {LABEL_CHOICES.map((choice, index) => (
              <button
                key={choice.value}
                type="button"
                className="rounded border border-black/15 px-3 py-2 text-left text-sm hover:bg-black/5 disabled:opacity-50 dark:border-white/20 dark:hover:bg-white/10"
                onClick={() => void choose(choice.value)}
                disabled={busy}
              >
                <span className="mr-2 font-mono text-xs text-neutral-500">{index + 1}</span>
                <span className="font-medium">{choice.name}</span>
                <span className="block text-xs text-neutral-600 dark:text-neutral-400">
                  {choice.hint}
                </span>
              </button>
            ))}
          </div>

          <div className="mt-3 flex flex-wrap gap-2 text-sm">
            <button
              type="button"
              className="rounded border border-black/15 px-3 py-1 disabled:opacity-50 dark:border-white/20"
              onClick={() => void choose(null)}
              disabled={busy}
            >
              <span className="mr-1 font-mono text-xs text-neutral-500">0</span> Can&apos;t tell —
              skip
            </button>
            <button
              type="button"
              className="rounded border border-black/15 px-3 py-1 disabled:opacity-50 dark:border-white/20"
              onClick={() => void undo()}
              disabled={busy || lastId === null}
            >
              <span className="mr-1 font-mono text-xs text-neutral-500">U</span> Undo last
            </button>
          </div>
        </article>
      )}
    </main>
  );
}
