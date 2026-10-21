# Library architecture: typed core, APIs, crates, builds

How icepool-rs is built: the typed core and its dynamic `Value` layer, the public Rust API, the error model, the
crates and their features, the TypeScript API, the documentation, and what the production builds ship. Decided in
task D1 (2026-10-17) and built by tasks C1-C8 and their follow-ups (C1b-C7b), all done by 2026-10-18; section 9 is
that history in brief. Background: docs/ANALYSIS.md (what icepool is), docs/reviews/2026-10-17-library-review.md (the
code-quality findings Q1-Q10 that motivated this design), docs/MODULE-MAP.md (which file ports which upstream
module) and docs/FACADE-AUDIT.md (what of the Python facade still runs in Python).

**Invariants.** Parity with the oracle is absolute for the native runner and both facades: same outcomes, order,
types, quantities, and raises. Readability over performance: every choice below is the clearer one, not
the faster one. When this document and the code disagree, the code is right and this document is out of date: fix
it.

---

## 1. Outcomes: `Ord` for typed outcomes, Python's order only in `Value`

**The generic algorithms are written against one small sealed trait, `OutcomeOrder`, whose comparison error is an
associated type** (`crates/icepool-core/src/outcome.rs`). Every `Ord` type gets it by a blanket impl with
`Incomparable = Infallible`; `Value` implements it with Python's semantics and `Incomparable = Error`. The
user-facing bound is `Outcome`, which is just "an `OutcomeOrder` that cannot fail", i.e. any `Ord` type.

```rust
/// What the core's algorithms need from an outcome: hashing, equality, and an order that may refuse.
pub trait OutcomeOrder: Clone + Eq + Hash + Debug + Send + Sync + 'static + sealed::Sealed {
    /// Why two outcomes could not be compared: `Infallible` for `Ord` types.
    type Incomparable: Into<Error>;
    fn try_lt(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    fn try_le(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    /// The truth of `self == other` as a comparison asks it (not dict-key equality: see `Value`).
    fn try_eq(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    /// Sorts `(outcome, tie)` items stably. `Ord` types: `slice::sort_by`. `Value`: CPython's timsort over `<`
    /// only (value/sort.rs), because an inconsistent `__lt__` must sort exactly as CPython sorts it.
    fn try_sort<T>(items: &mut Vec<(Self, T)>, tie_lt: impl Fn(&T, &T) -> bool) -> Result<(), Self::Incomparable>;
    /// Whether this may be a key at all (`Counts` rejects `None` and `Reroll`). `Ord` types: always.
    fn check_key(&self) -> Result<(), Self::Incomparable>;
}

/// An outcome type with a total order: any `Ord` type. This is the bound the typed API uses.
pub trait Outcome: OutcomeOrder<Incomparable = Infallible> + Ord {}

impl<T: Clone + Ord + Hash + Debug + Send + Sync + 'static> OutcomeOrder for T { type Incomparable = Infallible; /* ... */ }
impl OutcomeOrder for Value { type Incomparable = Error; /* value/order.rs: Python's `<`, which can raise */ }
```

What follows from it:

- **Generic code is written once, against `O: OutcomeOrder`**, returning `Result<T, Error>` (it also fails for
  reasons that are not comparisons). `?` converts `O::Incomparable` via `Into<Error>`.
- **The typed API is `impl<O: Outcome> Die<O>`** and unwraps the infallible comparisons with
  `outcome::infallible`. Comparisons on `Die<i64>` never return `Result`. (A blanket impl over `Ord` plus a separate
  impl for the local, non-`Ord` `Value` is coherent.)
- **`py_*` names exist only inside `src/value/`.** No generic code calls them (C1b made
  `grep -rn 'py_' crates/icepool-core/src --include=*.rs | grep -v '/value/'` empty, comments citing Python names
  excepted).
- `Value`'s `Eq`/`Hash` are dict-key semantics (`True == 1`, NaN unequal to itself: a known violation of `Eq`'s
  reflexivity, documented on `Value`). `try_eq` is Python's `bool(a == b)`, which differs for `Symbols`.
- **`OutcomeArith: OutcomeOrder`** is the arithmetic the generic algorithms perform (`n @ die`, sums, explode,
  `Again`, the sum evaluator): checked add, sub, neg, mul, `outcome * count` and zero. Implemented for the primitive
  integers (overflow is `Error::Overflow`) and for `Value` with Python's operators. Tuples and arrays get none:
  Python's tuple `+` concatenates, so no Rust meaning is the obvious one.
- `ToRatio` (the integer types and `Ratio`) is what the exact typed statistics need (section 2).

## 2. Generic features: evaluators, map, explode/Again, statistics, formatting

Principle: **a feature is generic over the smallest trait providing exactly the Python operations its source
performs**; Python behaviour that has no typed meaning lives in the `Value` layer (`src/value/`).

### Evaluators

`MultisetEvaluator<O>` (`evaluator/mod.rs`) is the core trait. The built-ins (`evaluator/{basic, keep, comparison,
lexi, argsort, poker}.rs`) are generic over `O: OutcomeOrder` (or `OutcomeArith` where they do outcome arithmetic)
with **typed states and finals**: `largest_count -> Int`, `highest_outcome_and_count -> (O, Int)`, comparisons
`-> bool`, `expand -> Vec<O>`, argsort and lexi `-> Vec<Int>` / `Int`. The typed entry points are
`Pool::evaluate`, `Deal::evaluate` and `PoolMixture::evaluate`, returning `Result<Die<E::Final>>` for an
`Outcome` final.

```rust
let largest: Die<Int> = d(6).pool(3).evaluate(&LargestCountEvaluator).unwrap();
```

- **`CountFold`** (`evaluator/fold.rs`) is the shape most built-ins share: fold `(outcome, counts)` into a state, read
  the result off the final state, no sizes. `count_fold_evaluators!(A, B<F>, ...)` writes the shared
  `MultisetEvaluator` impl for each listed type (not a blanket impl: that would make every other generic
  `impl MultisetEvaluator<O>` a coherence error).
- **The `Value` layer sees what Python sees.** Python builds the result die from the final outcomes *as Python
  values*, merging equal ones with Python semantics. `value::evaluators::PyEvaluator<E>` wraps a typed evaluator as
  a `MultisetEvaluator<Value>` and converts each final outcome with `IntoValue` (for `Int`, `bool`, tuples,
  `Vec<T>`, `Option<T>`, `Value`) before the die is built, never after.
- **Tuple counts (`MultiDeal`)** are Python dynamism: Python's arithmetic simply runs on a tuple count. The core
  trait passes int counts to `next_state` and answers `Error::Unported` for a tuple; the evaluators Python lets run
  on tuples implement `value::evaluators::TupleCounts` (next_state over `Value` counts and a `Value` state), which
  `PyEvaluator` switches to at the first tuple count.
- **User evaluators** written in Python or JS are `MultisetEvaluator<Value>` implementations in each binding
  (`icepool-py/src/evaluate.rs`, `icepool-wasm/src/api/evaluator.rs`; the runner has its own in
  `icepool-harness/src/host.rs`), calling the host object back.

### map, reroll, explode, Again, map_tools

```rust
impl<O: Outcome> Die<O> {
    pub fn map<P: Outcome>(&self, f: impl FnMut(&O) -> P) -> Die<P>;                         // die.rs
}
impl<O: OutcomeOrder> Die<O> {                                                                // die_map.rs
    pub fn map_die<P: OutcomeOrder>(&self, f: impl FnMut(&O) -> Die<P>) -> Result<Die<P>>;
    /// The general form the `Value` layer uses: an outcome, a die, `Reroll`; `Break`/`Restart` under `repeat`.
    pub fn try_map<P, E>(&self, /* f: FnMut(&O) -> Result<Mapped<P>, E>, ... */) -> Result<Die<P>, E>;
    pub fn reroll(&self, which: impl FnMut(&O) -> bool, depth: Depth) -> Result<Self>;
    pub fn filter(&self, which: impl FnMut(&O) -> bool, depth: Depth) -> Result<Self>;
}
pub enum Mapped<P> { Outcome(P), Die(Die<P>), Reroll, Break(Option<Target<P>>), Restart }
```

- `map_tools/` (repeat, absorbing Markov chains, `map_and_time`) is generic over `O: OutcomeOrder`. Python's
  argument handling (`star` inference, several dice as cartesian arguments, a `dict` repl, `time_limit` defaults)
  is facade work: the bindings and the runner build the closure.
- **`Again`** (`again/`): the engine is generic; `again/typed.rs` is the typed form, an expression over
  `O: OutcomeArith` with `std::ops` on it (`Again::new() + 1`), in depth and count modes, and `Die::explode` (which
  is depth-mode `Again`). Python's `Again` takes any operator and any object; that dynamic form is the bindings'.
- `to_pool.rs` (typed `explode_to_pool` / `reroll_to_pool`, returning a `PoolMixture` of keep generators from
  `mixture.rs`) and `expand.rs` are generic too; `value/to_pool.rs` and `value/expand.rs` keep Python's dynamic
  cases.

### Statistics

Exact and typed, on `impl<O: Outcome + ToRatio> Die<O>` (`statistics.rs`): `mean() -> Ratio`, `variance() -> Ratio`,
`probability(Comparison, &O) -> Ratio`, `standard_deviation() -> f64`. This matches Python for `int` dice
(Fraction results). The Python-typed versions (float vs Fraction by outcome type, `percent`, Python's `round`) are
the extension trait `value::PyStatistics`.

**Extension traits for Python behaviour.** Methods that only make sense with Python values are traits in `value/`
(`PyDie`, `PyStatistics`) implemented for `Die<Value>`. Rust resolves `die.mean()` to the inherent typed method for
`Die<i64>` and to the trait for `Die<Value>` (inherent candidates with unmet bounds are skipped). Associated
functions without `self` must not share names: rustc reports them ambiguous.

### Formatting

`impl<O: FormatOutcome> Display for Die<O>` prints icepool's default `str(die)` (the markdown table: outcome,
quantity, probability); `Pool`, `Deck`, `Deal`, `Int` and `Error` have `Display` too. The format-spec engine
(`format.rs`: markdown, csv, bbcode, html, col specs) is generic over `FormatOutcome`, implemented for the primitive
integers, `bool`, `Ratio`, `str` and `String`, `[T; N]`, `Vec<T>` and tuples (one column each, as Python splits
tuple outcomes), and `Value` (Python's `str`, `value/format.rs` over `value/pystr.rs`).

## 3. Errors: domain variants in core, exception names in the bindings

`crates/icepool-core/src/error.rs`: one `#[non_exhaustive] enum Error` of domain variants, grouped as arguments
(`InvalidArgument`, `WrongType`, `MissingAttribute`, `IndexOutOfRange`, `MissingKey`), outcomes (`Incomparable
{ op, left, right }`, `Unhashable { type_name }`, `DivisionByZero`), dice and evaluation (`ConflictingOrder`,
`UninferrableSize`, `UnsupportedOrder`), limits (`RecursionLimit`, `Overflow`, `TooLarge`), coverage
(`NotImplementedUpstream`: icepool itself raises `NotImplementedError`; `Unported`: Python answers, the port does
not) and callbacks (`Callback(HostError)`: raised by a host callback, re-raised by the host unchanged). `Display`
is the message alone, Python's own text where Python has one (upstream tests and the facade may match on it).

- **One variant, one Python exception, always.** A domain condition Python raises as different exception types at
  different sites is two variants, so the mapping is a total `match` with no message inspection.
- **The exception mapping lives in the bindings, not in core:** icepool-py maps each variant to a Python exception
  (`ConflictingOrder` to icepool's `ConflictingOrderError`; `Callback` re-raises the original `PyErr`) in
  `src/convert.rs`; icepool-wasm maps them to `IcepoolError` with the variant as `kind` (`src/api/error.rs`); the
  runner names the exception for wire results in `icepool-harness/src/wire.rs` (fixtures check only that both sides
  raised; the name is informational). The runner's emulations of Python builtins (`StopIteration`, unpack lengths,
  `dict` updates) are its own error type, `PErr`, never core variants. `Unported` reaches the harness as
  `unimplemented: ...`, which never counts as a match.
- **Panics vs `Result` in the typed API:** a programmer error Python raises as `ValueError` on an argument the
  caller controls (`d(0)`, overflow in an operator) panics with Python's message, like `slice::chunks(0)`, and
  every such convenience documents it under *Panics* and has a fallible form (`try_d`, `try_z`, `Die::try_binary`).
  Everything the caller cannot cheaply pre-check (order conflicts, callback errors, `TooLarge` sizes) returns
  `Result`. The `Value` layer always returns `Result`.

## 4. Public Rust API

What docs.rs shows for `icepool-core` (`src/lib.rs`); every other module is private and reached only through these:

```rust
pub use again::{Again, AgainEnd, AgainOutcome};   pub use die::{Depth, Die};
pub use deal::Deal;  pub use deck::Deck;  pub use multi_deal::MultiDeal;  pub use pool::Pool;
pub use mixture::{KeepGenerator, PoolMixture};     pub use to_pool::RerollMode;
pub use die_map::{Mapped, Target};                 pub use die_ops::IntoDie;
pub use function::{d, z, try_d, try_z, pointwise_max, pointwise_min};
pub use outcome::{Outcome, OutcomeArith, OutcomeOrder, ToRatio};  pub use format::FormatOutcome;
pub use population::Comparison;  pub use order::Order;  pub use error::{Error, Result};
pub use int::Int;  pub use value::Value;           // plus `Quantity` (dashu UBig) and `Ratio` (dashu RBig)
pub mod evaluator;   // MultisetEvaluator, CountFold, the built-in evaluators, sources, Final
pub mod expression;  // multiset expressions and their operators
pub mod value;       // the dynamic outcome with Python's semantics and the Python-only APIs, for language bindings
pub mod host;        // host-callback traits, for language bindings
pub mod random;      // CPython's Mersenne Twister, for `sample` in the bindings and the runner
```

A binding that needs something gets a deliberate public API in `value` or `host`, documented as being for language
bindings, never a widened `pub mod`; there is no `#[doc(hidden)]`.

The typed surface (the crate guide, `crates/icepool-core/README.md`, shows each with a doctest):

```rust
use icepool_core::{d, Die};

let d6: Die<i64> = d(6);                         // panics "sides must be at least 1." for d(0); try_d returns Err
let two = &d6 + &d6;                             // std::ops on Die and &Die, and Die op O
let best3 = d6.pool(4).highest(3).sum();         // Pool<i64> -> Die<i64>
let hits: Die<bool> = d(20).ge(11);              // comparisons are methods: lt le gt ge equal not_equal
let three_d6 = d6.sum_rolls(3);                  // Python's 3 @ d6
let mean = two.mean();                           // Ratio, exact: 7
println!("{two}");                               // icepool's markdown table
Die::from_iter([1, 2, 2, 3]);  Die::from_weights([(1, 2u32), (2, 1)]);  Die::from(1..=6);  Die::single(5);
```

- `std::ops` for `Die<O>` where `O` has the operator, on owned and borrowed dice and with an `O` on the right;
  `i64 + Die<i64>` etc. by macro for the primitive integers (orphan rule). Overflow panics as the checked operator
  would; `Die::try_binary` is the fallible form.
- Comparisons that return dice are methods; `eq`/`ne` stay `PartialEq`'s, which is Python's `die.equals`. Python
  operators without a Rust one get names: `@` is `sum_rolls`, `//` is `floor_div`, `**` is `pow`.
- `#![warn(missing_docs)]`, `#![warn(missing_debug_implementations)]`, and a doctest on the public items (C7b,
  C3e; `scripts/doctest-coverage.py` measures it).

## 5. Crates, host callbacks, feature flags

```text
crates/
  icepool-core/     the library. Deps: dashu-*, rustc-hash. No features, no serde_json, no runner, no binary.
  icepool-py/       PyO3 extension `icepool_rs` behind python/icepool. Depends on icepool-core only.
  icepool-wasm/     wasm-bindgen exports (src/api/) under the TypeScript package in js/. Production: icepool-core
                    only; its test-only `runner` feature adds the crate below.
  icepool-harness/  test-only, unpublished: the JSON program runner (src/, nodes in ops/ and expression/), the
                    `icepool-run` binary harness/core/tool.py drives, tests/fixtures.rs, which runs every
                    fixture, and khepri's Face to Face engine (src/f2f.rs: application code on the typed Die<i64>,
                    the runner's `khepri` node). Depends on icepool-core and serde_json.
```

- **The runner is a client of the public API** (since C5b and C3d): it uses `icepool_core::{..., evaluator,
  expression, value, host}` exactly as the bindings do, which is what keeps that API complete. It is the
  Python-dynamic layer in miniature: it unpacks Python-style arguments and calls the static core.
- **Host callbacks** (`icepool_core::host`): `HostObject` / `HostRef` / `HostError` (an outcome of a host type, held in
  `Value::Host`, and the host's own exception), `HostSourceObject` / `HostSource` (a user's `MultisetSource`),
  `HostOperator` (a user's `MultisetOperator`), `Traverse` / `owned_hosts` (for the host's cycle collector) and
  `set_interrupt_check` (Ctrl-C, polled by the core's long loops). Their arguments and results are core types only.
  Callables and evaluators belong to each binding (section 2, "User evaluators"): the core never names a host
  language.
- **Feature flags**, none default:
  - `icepool-wasm/runner` (test-only): the `run` export that `run-wasm.sh` drives in the long gate, and the JSON-op
    bridge (`src/facade.rs`, `call` / `from_wire` / `wire_result`) under `js/src/rpc.ts`, through which the JS fixture
    tests reach the nodes the TypeScript API has no counterpart for (khepri, complex numbers, catalogue callables).

  No feature for `Value`: it is part of the core (both bindings and the runner need it), but no generic code names
  it (section 1).
- **Test-only code** sits in `#[cfg(test)]`, `[dev-dependencies]`, `tests/`, the harness crate, or behind
  `icepool-wasm/runner`. The Python package's tests (`crates/icepool-py/tests`), the run-facade scripts and the JS
  tests (`js/test`, plus `js/src/rpc.ts` and `js/src/internal.ts`) are outside the shipped file lists (section 8).

## 6. The JavaScript / TypeScript API

**A TypeScript API written in TypeScript (`crates/icepool-wasm/js/src/*.ts`, compiled by tsc; the `.d.ts` is
generated), over typed wasm-bindgen exports of the core** (`WasmDie`, ... in `icepool-wasm/src/api/`, taking and
returning `JsValue`s converted by `src/api/convert.rs` straight to `Value`, no JSON). Neither the program runner nor
the JSON-op bridge is in the package (C6l).

Conventions, chosen to be predictable from the Python docs:

| Python | TypeScript |
|---|---|
| names `snake_case` | **camelCase** (`keep_outcomes` -> `keepOutcomes`), JS's convention; classes keep Python's names |
| operator `__op__` / `__rop__` | method named after the dunder: `add sub mul truediv floordiv mod pow matmul neg pos abs invert and or xor lshift rshift lt le gt ge eq ne`, reflected `radd rsub rmul rtruediv rfloordiv rmod rpow rmatmul rand ror rxor rlshift rrshift` |
| positional / positional-or-keyword params | positional |
| keyword-only params (after `*`) | trailing options object: `die.explode(which, { depth: 9, end })` |
| `*args` | rest parameters |
| `int` | in: `number` (safe integer) or `bigint`; out: `number` when safe, else `bigint` |
| quantities, denominators | always `bigint` |
| `float` | `number`; an integral float result is `Float` (so `2.0` stays a float when passed back) |
| `Fraction`, `complex` | `Fraction` (bigint parts), `Complex` |
| `tuple` / `Vector` / `Symbols` | readonly array / `Vector` / `Symbols` |
| `None`, `Reroll`, `Restart`, `Break(x)` | `null`, `Reroll`, `Restart`, `new Break(x)` |
| exceptions | `IcepoolError extends Error` with `kind` (the core variant name) and `pythonType`; subclasses `ConflictingOrderError`, `UnsupportedOrder`; a callback's exception is rethrown as itself |
| iteration over a die (keys) | `[Symbol.iterator]` yields outcomes; `items()` yields `[outcome, quantity]`, `quantities()` |
| `str(die)`, `repr(die)` | `toString()`, `repr()` |
| object lifetime (GC) | `free()` and `[Symbol.dispose]` (so `using die = d(6)` works); a `FinalizationRegistry` frees what is forgotten |

The API mapping, by example (all of `icepool/__init__.py` is covered the same way):

| Python icepool | TypeScript |
|---|---|
| `d(6)`, `z(6)`, `coin(1, 3)`, `one_hot(4)` | `d(6)`, `z(6)`, `coin(1, 3)`, `oneHot(4)` |
| `Die([1, 2, 2])`, `Die({1: 2, 2: 1})`, `Die([1, 2], times=[2, 1])` | `new Die([1, 2, 2])`, `new Die(new Map([[1, 2n], [2, 1n]]))`, `new Die([1, 2], { times: [2, 1] })` |
| `d6 + d8`, `3 - d6`, `d6 > 3`, `3 @ d6`, `d6 @ d6` | `d6.add(d8)`, `d6.rsub(3)`, `d6.gt(3)`, `d6.rmatmul(3)`, `d6.matmul(d6)` |
| `d6 == d6` (a die with a truth value) / `d6.equals(d6)` | `d6.eq(d6)` (a `Die`, `.truth()` for its bool) / `d6.equals(d6)` |
| `d6.map(lambda x: x * 2)`, `d6.map(f, d8, star=True)` | `d6.map(x => x * 2)`, `d6.map(f, d8, { star: true })` (`star` inferred from `f.length` as Python counts parameters) |
| `d6.reroll([1])`, `d6.explode(depth=3)`, `d6.keep(3, 2)` | `d6.reroll([1])`, `d6.explode(null, { depth: 3 })`, `d6.keep(3, 2)` |
| `d6.pool(4).highest(3).sum()`, `Pool([d6, d8])[-2:]` | `d6.pool(4).highest(3).sum()`, `new Pool([d6, d8]).keep(-2, null)` (`[a:b]` is `keep(a, b)`, `[i]` is `keep(i)`) |
| `d6.pool(5).largest_count()`, `.all_straights()` | `d6.pool(5).largestCount()`, `.allStraights()` |
| `Deck([1, 2, 3]).deal(2)`, `MultiDeal`, `Deal` | `new Deck([1, 2, 3]).deal(2)`, `MultiDeal`, `Deal` |
| `Die([Again + 1, 1])`, `Again` expressions | `new Die([Again.add(1), 1])`, `Again` is an expression object with the same methods |
| `class E(MultisetEvaluator): def next_state(self, s, order, outcome, *counts)` | `class E extends MultisetEvaluator { nextState(s, order, outcome, ...counts) }`; also `initialState`, `extraOutcomes`, `finalOutcome` |
| `E().evaluate(pool)`, `pool.evaluate(E())` | `new E().evaluate(pool)`, `pool.evaluate(new E())` |
| `@multiset_function def f(a, b): return (a & b).size()` | `const f = multisetFunction((a, b) => a.and(b).size()); f.evaluate(p, q)` (traced once, as in Python) |
| `die.mean()`, `.probability(3)`, `.probabilities()`, `.quantity('<=', 3)` | `die.mean()`, `.probability(3)` (`number`; `{ exact: true }` gives a `Fraction`), `.probabilities()`, `.quantity('<=', 3)` |
| `die.format('md')`, `f'{die:md:o|q|%==}'` | `die.format('md')`, `die.format('md:o\|q\|%==')` |
| `icepool.Order.Ascending`, `icepool.Reroll` | `Order.Ascending`, `Reroll` |

- **Where the work is done.** As in the Python facade, where upstream's own Python does the argument juggling and
  calls `icepool_rs` for the work, the TypeScript does Python's juggling and calls the typed exports for the work:
  `die.reroll(which)` calls the JS predicate on each outcome itself (Python's `_select_outcomes`) and passes the core
  a mask; `map(f, ...)` enumerates the joint outcomes in JS and builds the result with a `DieBuilder`; pools and
  multiset expressions are TypeScript objects that one export turns into the core's tree at evaluation (as
  icepool-py's `evaluate.rs` does). The core never calls those callbacks, so they cannot trap inside it. Callbacks
  the core must call during its own work (JS evaluators, `mapCounts`) are host callbacks
  (`src/api/evaluator.rs`).
- **Evaluator states in JS** must be hashable the Python way: primitives, `Fraction`, `Float`, and (nested)
  readonly arrays of them, converted to `Value` once per call. Anything else is a `TypeError`-kind `IcepoolError`
  ("unhashable"), as Python raises for a list state. This is the one genuine JS constraint (ANALYSIS.md).
- **No `any` in the public types.** `Outcome` is a union type; `Die<T extends Outcome = Outcome>` is generic so
  `d(6)` is `Die<number>` in the types.
- **What the runner does not model raises its `NotImplementedError`** (kind `Unported`) in the API too, rather than
  computing an answer the port has nowhere else (docs/NOTES.md, "C6l").
- **Tests.** `js/test/fixtures.mjs` is a test-only adapter that translates every fixture program into calls of this
  public API (the few nodes with no public counterpart go through the bridge, section 5); `js/test/upstream-*.mjs`
  mirror upstream's Python tests; `api.test.mjs` checks that no public module imports the bridge. `js/build.sh
  --test` runs them all (the long gate and CI).

## 7. Documentation

- **Rust:** rustdoc. The crate-level guide is `crates/icepool-core/README.md`, included by
  `#![doc = include_str!("../README.md")]`: first dice, operators, reading a die, statistics, pools and evaluators,
  map/reroll/explode/`Again`, decks, exact quantities, typed outcomes vs `Value`, errors and panics. Examples are
  doctests and run with `cargo test`. `RUSTDOCFLAGS=-D warnings cargo doc --no-deps -p icepool-core` runs in
  `.github/workflows/ci.yml`. Provenance comments cite the Python file and function (no line numbers).
- **Python:** upstream's documentation is the documentation: `python/icepool` keeps upstream's docstrings verbatim,
  so upstream's rendered docs apply. `python/README.md` describes the package; `python/DIFFERENCES.md` lists what
  differs on the Rust backend, written by hand from DIVERGENCES.md, docs/NOTES.md and docs/FACADE-AUDIT.md (keep it
  current when one of those changes).
- **JS/TS:** `crates/icepool-wasm/js/README.md` and the guide `js/docs/guide.md` (install, first dice, pools and
  evaluators, decks, Again/explode, custom evaluators in JS, BigInt quantities, disposing, differences from Python
  icepool), TSDoc on every exported symbol, and an API reference generated by TypeDoc (`npm run docs`). Every
  fenced `ts` example in the guide and the TSDoc is executed by `js/test/doc-examples.test.mjs`. typescript and
  typedoc are devDependencies of the JS package only. The package is not published to npm yet: build it from the
  repository (docs/DEVELOPMENT.md).
- **The repository:** README.md (upstream icepool's README with a note on the port), docs/DEVELOPMENT.md (the three
  bindings, the parity story, layout, commands),
  docs/MODULE-MAP.md (one row per upstream module; `scripts/upstream_changes.py OLD NEW` parses it to list what to
  revisit when upstream changes) and docs/FACADE-AUDIT.md.

## 8. Production builds contain no test code

What ships, neither containing tests, fixtures, the runner, harness code, catalogue functions or JSON-program code:

- **The Python wheel**, built by `scripts/build-wheel.sh OUT_DIR`: the `icepool` facade package plus the
  `icepool_rs` extension, on the Rust backend when `ICEPOOL_BACKEND` is unset. maturin packs only the Python
  sources inside its project directory and follows no symlinks out of it, so `python-source = "../../python"`
  cannot work; the script stages a project in a temporary directory from `crates/icepool-py/wheel/` (its
  pyproject.toml, with `manifest-path` back to the crate, and `icepool_rs/__init__.py`) plus a copy of
  `python/icepool` and the build-time constant `icepool/_wheel.py` (`DEFAULT_BACKEND = 'rust'`, in the wheel only),
  and runs `maturin build --release` there. `crates/icepool-py/pyproject.toml` builds the extension alone: that is
  what the harness's `maturin develop` installs, choosing the backend explicitly.
- **The npm package**, `npm pack` of `crates/icepool-wasm/js` as `js/build.sh` builds it (node and web builds of
  the wasm module, no features); package.json `files` leaves out `dist/internal.*` and `dist/rpc.*`.

`scripts/check-production.sh` checks both (the fast gate and CI's `production` job run it); a missing tool is a
failure, never a silent pass:

1. **Dependency graph:** `cargo tree -e normal,features` of icepool-py, and of icepool-wasm for
   `wasm32-unknown-unknown`, built alone as maturin and wasm-pack build them, contains none of icepool-harness,
   serde_json.
2. **Wheel contents** (`scripts/build-wheel.sh`, `unzip -Z1`): no path matching tests, conftest, fixtures, harness,
   catalogue, program, run-facade, run-wasm or bench; `icepool/__init__.py`, `icepool/_wheel.py` and
   `icepool_rs/__init__.py` present. Then a smoke test: the wheel installed into a fresh venv, run with `-I` from
   outside the checkout and `ICEPOOL_BACKEND` unset, computes `icepool.d(6) + 1` on the Rust backend.
3. **The extension module:** no `icepool-harness`, `icepool_harness`, `src/program/` or `serde_json` strings (panic
   locations embed source paths, so any linked runner code shows) and no `icepool_harness::` or `serde_json::`
   symbols.
4. **npm contents** (`js/build.sh --dev`, `npm pack`, `tar tzf`): no test-only paths as in 2, and no `rpc` or
   `internal` module. (`--dev` skips wasm-opt, which only shrinks the module.)
5. **The wasm modules:** none of the strings of 3, and none of the exports `run`, `calculate`, `call`, `from_wire`,
   `wire_result` (read with node's `WebAssembly.Module.exports`).

It prints what it checked (file lists, sizes, exports) and exits non-zero with the offending paths or strings.

## 9. How it got here

Tasks C1-C8 implemented this design in order, the gate green at every commit, each step either a move or a
semantic change with fixtures, never both. The runner drove the oracle comparison throughout: it runs on `Value`,
so every step that kept the `Value` instantiation's behaviour kept every fixture passing.

1. **C1, C1b: typed outcomes.** `OutcomeOrder` / `Outcome` / `OutcomeArith`; generic code switched module by
   module; `pysort.rs` became `value/sort.rs` and the `py_*` methods moved into `value/`.
2. **C2, C2b-C2g: generic features.** The evaluators (`CountFold` first, with poker; then basic, keep, comparison,
   lexi, argsort), map/map_tools, explode/`Again`, to_pool/expand, statistics (`ToRatio`), formatting
   (`FormatOutcome`), typed `explode_to_pool` / `reroll_to_pool` (C2g); khepri moved into `crates/icepool-khepri`
   (C2f), and later into the harness as test-only code (src/f2f.rs) once khepri-f2f built its own engine.
3. **C3, C3b-C3e: public API.** `std::ops`, typed constructors, `Display`, iterators, the curated root; the bindings
   (C3b) and then the runner (C3d) moved onto deliberate public APIs; doctests (C3c, C3e).
4. **C4: errors.** Domain variants, every site audited, the mapping moved into the bindings and the runner.
5. **C5, C5b, C5c: crates and production builds.** `crates/icepool-harness` with `icepool-run`; once the loop owner
   moved the protected fixtures test to `crates/icepool-harness/tests/fixtures.rs`, C5b moved the runner out of
   core's `program/` and dropped core's interim `runner` feature; `scripts/check-production.sh`; C5c made the wheel
   ship the facade (`scripts/build-wheel.sh`).
6. **C6, C6b-C6l: the TypeScript API.** C6 replaced the JS facade (a JSON-op RPC over the runner) with the
   TypeScript API at once, `Die` first on typed exports and the rest on an interim bridge; C6c-C6k moved the rest
   class by class (Die and the free functions, pools and expressions, JS evaluators and `multisetFunction`, decks
   and deals, the keep family, reflected operators); C6l took the bridge out of the package. C6b wrote the docs.
7. **C7, C7b: Rust tests.** Typed `Die<i64>` unit tests and doctests, and property tests comparing each typed path
   with the `Value` path on the same integer dice (`crates/icepool-core/tests/typed_vs_value.rs`), all under plain
   `cargo test`.
8. **C8: presentation.** README, docs/MODULE-MAP.md, CI (`.github/workflows/ci.yml`), `scripts/upstream_changes.py`,
   and the oversized files split by domain: the runner's `ops.rs` and `expression.rs` into
   `crates/icepool-harness/src/ops/` and `expression/`, core's `value/ops.rs` into `value/ops/`.
