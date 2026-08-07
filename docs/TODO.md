# TODO — deferred ideas

Things worth building that aren't worth building *now*. This is deliberately separate from
`PLAN.md`: that file is the committed phase-by-phase plan, this one is the parking lot for
ideas that came up mid-work and got consciously postponed.

Nothing here is a commitment. An entry earning its way out of this file is a decision to
make later, with whatever you know then.

## Format

Each entry keeps the same shape so this stays skimmable as it grows:

```
## <Short title>

**Status:** deferred — <what would make it worth revisiting>
**Area:** <file or subsystem>
**Added:** <YYYY-MM-DD>

<What it is, in a sentence or two.>

**Why not now:** <the reason it was postponed — the most important field, since future-you
won't remember the tradeoff that made this a "later">

**Sketch:** <rough approach, so you don't re-derive it from scratch>

**Watch out for:** <known traps, cost, or interactions>
```

---

## Out-of-order token matching

**Status:** deferred — revisit if real queries start missing, or after the fuzzy tier lands
**Area:** `apps/fridge-app/backend/src/nlp.rs`, prefix/scoring tiers
**Added:** 2026-08-07

Match a query's tokens against a candidate's tokens regardless of the order they're typed
in, so `milk almond` finds "Almond milk" and `oil olive` finds "Olive oil". Today both
queries return nothing: whole-string prefix fails because the string doesn't start that
way, and any-token prefix fails because a multi-word query can never prefix-match a single
token.

**Why not now:** people type left to right, so it's rare in practice. Whole-string prefix
already covers in-order multi-word queries for free (`almond mi` *is* a prefix of
`almond milk`), which is the overwhelmingly common case. This tier only earns its keep for
genuinely reordered input, and it's the most expensive and false-positive-prone of the
prefix variants — bad ratio while the cheap tiers aren't finished yet.

**Sketch:** split both query and candidate on whitespace. Require *every* query token to
prefix-match *some* candidate token, each candidate token consumed at most once. Score
below the in-order prefix sub-band, since word order is real evidence about intent and
throwing it away should cost something. Aggregate by total matched length over candidate
length, consistent with how coverage is measured elsewhere.

**Watch out for:**
- Cost is O(query_tokens x candidate_tokens) per candidate — the only tier that isn't
  linear in the candidate. Fine at 466 entries, worth remembering if the catalog grows.
- Long multi-token catalog names ("Roasted nuts (peanuts, cashews, almonds)") have many
  tokens and will match loosely. Consider requiring a minimum coverage before accepting.
- Needs a clear tie-break against the plain prefix tiers, or an out-of-order match can
  outrank an in-order one and the ranking stops being explainable.
- `rapidfuzz`'s `token_set_ratio` implements this class of matching if adopting beats
  building — worth a look before writing it by hand.
