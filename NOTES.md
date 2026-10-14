# Notes for the next iteration

Gotchas, decisions and layout notes, written by the iterations themselves. Keep it current: fix or delete what
turns out wrong. Newest decisions at the top of each section.

## Decisions

## Gotchas

- The oracle drops zero quantities passed to `Die({...})` at construction: `Die({1: 0, 3: 5})` has one outcome.
  Zero quantities can still arise elsewhere (see docs/semantics.md); trust the oracle, not this note.
