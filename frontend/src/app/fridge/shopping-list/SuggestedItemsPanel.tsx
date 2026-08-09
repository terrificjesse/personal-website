import type { ShoppingSuggestion } from "@/lib/shoppingListApi";

const REASON_LABEL: Record<ShoppingSuggestion["reason"], string> = {
  frequently_purchased: "you buy this often",
  expiring_replacement: "expiring soon",
};

/**
 * Separate from the manual list per PLAN.md. Always empty right now — the backend's
 * `suggest_shopping_items` ([learn]) is a stub that returns `[]` until implemented, so this
 * panel has nothing to render yet. That's expected, not a bug.
 */
export function SuggestedItemsPanel({
  suggestions,
  onAccept,
}: {
  suggestions: ShoppingSuggestion[];
  onAccept: (suggestion: ShoppingSuggestion) => void;
}) {
  if (suggestions.length === 0) {
    return (
      <p className="mt-6 text-sm opacity-60">
        No suggestions right now.
      </p>
    );
  }

  return (
    <ul className="mt-6 space-y-2">
      {suggestions.map((suggestion) => (
        <li
          key={suggestion.item_name}
          className="flex items-center justify-between gap-4 rounded border border-black/10 px-3 py-2 dark:border-white/10"
        >
          <div>
            <p className="text-sm font-medium capitalize">{suggestion.item_name}</p>
            <p className="text-xs opacity-60">{REASON_LABEL[suggestion.reason]}</p>
          </div>
          <button
            onClick={() => onAccept(suggestion)}
            className="text-xs text-blue-600 hover:underline"
          >
            Add to list
          </button>
        </li>
      ))}
    </ul>
  );
}
