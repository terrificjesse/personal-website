"use client";

import { useEffect, useId, useRef, useState } from "react";
import { fetchSuggestions, type Suggestion } from "@/lib/fridgeApi";

const DEBOUNCE_MS = 120;
const SUGGESTION_LIMIT = 5;

type Props = {
  value: string;
  onChange: (value: string) => void;
  onSelect: (suggestion: Suggestion) => void;
  inputId: string;
  placeholder?: string;
};

/**
 * Typeahead for item names.
 *
 * Deliberately does *not* preselect a suggestion: `activeIndex` starts at -1 and resets on
 * every keystroke, so Enter always commits exactly what was typed unless the user arrows
 * onto a suggestion first. That costs one keypress on the common path and makes it
 * impossible to silently add the wrong item by typing fast.
 */
export function ItemNameCombobox({ value, onChange, onSelect, inputId, placeholder }: Props) {
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);

  const listboxId = useId();
  // Set when a suggestion is picked, so the resulting `value` change doesn't immediately
  // refetch and reopen the list we just closed.
  const skipNextFetch = useRef(false);

  useEffect(() => {
    if (!open) return;

    if (skipNextFetch.current) {
      skipNextFetch.current = false;
      return;
    }

    const controller = new AbortController();
    const timer = setTimeout(() => {
      fetchSuggestions(value, { limit: SUGGESTION_LIMIT, signal: controller.signal })
        .then((results) => {
          setSuggestions(results);
          setActiveIndex(-1);
        })
        .catch((err) => {
          if (err instanceof DOMException && err.name === "AbortError") return;
          setSuggestions([]);
        });
    }, DEBOUNCE_MS);

    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [value, open]);

  function select(suggestion: Suggestion) {
    skipNextFetch.current = true;
    onSelect(suggestion);
    setOpen(false);
    setActiveIndex(-1);
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    switch (e.key) {
      case "ArrowDown": {
        e.preventDefault();
        if (!open) {
          setOpen(true);
          return;
        }
        // Wraps back to -1 (the raw text) rather than to the first item, so arrowing past
        // the end returns you to what you typed.
        setActiveIndex((i) => (i + 1 >= suggestions.length ? -1 : i + 1));
        return;
      }
      case "ArrowUp": {
        e.preventDefault();
        if (!open) {
          setOpen(true);
          return;
        }
        setActiveIndex((i) => (i - 1 < -1 ? suggestions.length - 1 : i - 1));
        return;
      }
      case "Enter": {
        // Only intercepted when the user has deliberately arrowed onto a suggestion.
        // Otherwise this falls through and the form submits the literal text.
        const active = activeIndex >= 0 ? suggestions[activeIndex] : undefined;
        if (open && active) {
          e.preventDefault();
          select(active);
        }
        return;
      }
      case "Escape": {
        setOpen(false);
        setActiveIndex(-1);
        return;
      }
      case "Tab": {
        setOpen(false);
        return;
      }
    }
  }

  const showList = open && suggestions.length > 0;
  const activeId = activeIndex >= 0 ? `${listboxId}-${activeIndex}` : undefined;
  const isEmptyQuery = value.trim() === "";

  return (
    <div className="relative">
      <input
        id={inputId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={() => setOpen(true)}
        // Delayed so a click on an option is processed before the list unmounts. The
        // options also preventDefault on mousedown, which stops the blur from firing at all
        // for mouse users; this covers focus lost by other means.
        onBlur={() => window.setTimeout(() => setOpen(false), 0)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        autoComplete="off"
        role="combobox"
        aria-expanded={showList}
        aria-controls={listboxId}
        aria-autocomplete="list"
        aria-activedescendant={activeId}
        className="w-56 rounded border border-black/15 px-2 py-1.5 text-sm dark:border-white/20 dark:bg-transparent"
      />

      {showList && (
        <div className="absolute left-0 top-full z-10 mt-1 w-72 overflow-hidden rounded border border-black/15 bg-background shadow-lg dark:border-white/20">
          {isEmptyQuery && (
            <p className="border-b border-black/10 px-2 py-1 text-[11px] uppercase tracking-wide opacity-50 dark:border-white/10">
              Recent
            </p>
          )}

          <ul id={listboxId} role="listbox">
            {suggestions.map((suggestion, index) => (
              <li
                key={`${suggestion.source}-${suggestion.name}`}
                id={`${listboxId}-${index}`}
                role="option"
                aria-selected={index === activeIndex}
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => select(suggestion)}
                className={`flex cursor-pointer items-center justify-between gap-3 px-2 py-1.5 text-sm ${
                  index === activeIndex ? "bg-black/10 dark:bg-white/15" : ""
                }`}
              >
                <span className="capitalize">{suggestion.name}</span>
                {suggestion.source === "fridge" && (
                  <span className="shrink-0 text-[11px] opacity-50">in fridge</span>
                )}
              </li>
            ))}
          </ul>

          {!isEmptyQuery && (
            <p className="border-t border-black/10 px-2 py-1 text-[11px] opacity-50 dark:border-white/10">
              {activeIndex >= 0
                ? "Enter to use the highlighted suggestion"
                : `Enter to add “${value.trim()}” as typed`}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
