# Library architecture: typed core, APIs, crates, builds

Status: decided (task D1, 2026-10-17). Tasks C1-C8 implement this document; where a C task and this document
disagree, this document wins and the task's details should be refined to match. Background:
docs/reviews/2026-10-17-library-review.md (code-quality findings 1-10 are referred to as Q1-Q10 below).

This file lives in `design/` because `docs/` is protected by the guard; QUESTIONS.md asks the loop owner to move it
to `docs/design/library-architecture.md`. Until then, "docs/design/library-architecture.md" in tasks.json means
this file.

**Invariants for every step below.** Parity with the oracle stays absolute for the native runner and the Python
facade: same outcomes, order, types, quantities, and raises. The gate stays green at every commit. Readability over
performance (AGENTS.md): every design choice below is the clearer one, not the faster one.

---

## 1. Outcomes: `Ord` for typed outcomes, Python's order only in `Value`

Today `Outcome` requires `py_lt/py_le/py_gt/py_ge/py_eq/check_key -> Result` (Q1), so every `Die<i64>` comparison
returns `Result`. Decision: **the generic algorithms are written against one small sealed trait, `OutcomeOrder`,
whose comparison error is an associated type.** Every `Ord` type gets it by a blanket impl with
`Incomparable = Infallible`; `Value` implements it with Python's semantics and `Incomparable = Error`. The
user-facing bound is `Outcome`, which is just "an `OutcomeOrder` that cannot fail", i.e. any `Ord` type.

```rust
// outcome.rs
use std::convert::Infallible;

/// What the core's algorithms need from an outcome: hashing, equality, and an order that may refuse.
///
/// Implemented for every `Ord + Hash + Clone + Debug + Send + Sync + 'static` type (a total order that cannot
/// fail) and for [`Value`], whose order is Python's `<` and can raise. Sealed: these are the only two.
pub trait OutcomeOrder: Clone + Eq + Hash + Debug + Send + Sync + 'static + sealed::Sealed {
    /// Why two outcomes could not be compared: `Infallible` for `Ord` types.
    type Incomparable: Into<Error>;

    /// `self < other`.
    fn try_lt(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    /// `self <= other` (`Die.clip`).
    fn try_le(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    /// The truth of `self == other` as a comparison asks it (not dict-key equality: see `Value`).
    fn try_eq(&self, other: &Self) -> Result<bool, Self::Incomparable>;
    /// Sorts `(outcome, tie)` items stably, ties by `tie_lt` among equal outcomes (`Counts` sorts `(key, value)`
    /// tuples). `Ord` types: `slice::sort_by`. `Value`: CPython's timsort over `<` only (value/sort.rs).
    fn try_sort<T>(items: &mut Vec<(Self, T)>, tie_lt: impl Fn(&T, &T) -> bool) -> Result<(), Self::Incomparable>;
    /// Whether this may be a key at all (`Counts` rejects `None` and `Reroll`). `Ord` types: always.
    fn check_key(&self) -> Result<(), Self::Incomparable>;
}

/// An outcome type with a total order: any `Ord` type. This is the bound the typed API uses.
pub trait Outcome: OutcomeOrder<Incomparable = Infallible> + Ord {}

impl<T: Clone + Ord + Hash + Debug + Send + Sync + 'static> OutcomeOrder for T {
    type Incomparable = Infallible;
    fn try_lt(&self, other: &Self) -> Result<bool, Infallible> { Ok(self < other) }
    // ... the rest likewise; try_sort is `items.sort_by(|a, b| a.0.cmp(&b.0).then(...))`.
}
impl<T: OutcomeOrder<Incomparable = Infallible> + Ord> Outcome for T {}

// value/order.rs: Python's semantics, the only non-total implementation.
impl OutcomeOrder for Value {
    type Incomparable = Error;
    fn try_lt(&self, other: &Self) -> Result<bool> { self.py_lt(other) }   // TypeError for int < str, host __lt__...
    fn try_sort<T>(items: &mut Vec<(Self, T)>, tie_lt: impl Fn(&T, &T) -> bool) -> Result<()> {
        sort::timsort(items, |a, b| if a.0.py_eq(&b.0)? { Ok(tie_lt(&a.1, &b.1)) } else { a.0.py_lt(&b.0) })
    }
    // ...
}
```

Rules that follow from it:

- **Generic code is written once, against `O: OutcomeOrder`**, returning `Result<T, Error>` (it also fails for
  reasons that are not comparisons). `?` converts `O::Incomparable` via `Into<Error>`.
- **The typed API is `impl<O: Outcome> Die<O>`** and unwraps the infallible comparisons with one helper,
  `fn infallible<T>(r: Result<T, Infallible>) -> T { match r { Ok(t) => t, Err(e) => match e {} } }`. Comparisons on
  `Die<i64>` never return `Result`. (A blanket impl over `Ord` plus a separate impl for the local, non-`Ord` `Value`
  is coherent; checked with rustc for this design.)
- **`py_*` names exist only inside `src/value/`.** `pysort.rs` becomes `value/sort.rs` (it exists because an
  inconsistent `__lt__` must sort exactly as CPython does). No generic code calls `py_*`; `grep -rn 'py_' src --include=*.rs
  | grep -v '^src/value/'` is empty after C1 (comments citing Python names excepted).
- `Value`'s `Eq`/`Hash` stay dict-key semantics (`True == 1`, NaN unequal to itself: a known violation of `Eq`'s
  reflexivity, documented on `Value`). `try_eq` is Python's `bool(a == b)`, which differs for `Symbols`.
- `SumOutcome` is replaced by **`OutcomeArith: OutcomeOrder`**, the arithmetic the generic algorithms perform
  (`n @ die`, sums, explode, `Again`, the sum evaluator): `try_add`, `try_sub`, `try_neg`, `try_mul`,
  `try_mul_count(&Int)` (`outcome * count`), `try_zero` (`x * 0`, `Population.zero`). Implemented by a macro for
  `i8..=i128, isize` with checked operations (`Error::Overflow`) and for `Value` with Python's operators. Tuples and
  arrays get no `OutcomeArith`: Python's tuple `+` concatenates, so no Rust meaning is the obvious one.

## 2. Generic features: evaluators, map, explode/Again, formatting

Principle: **a feature is generic over the smallest trait providing exactly the Python operations its source
performs**; Python behaviour that has no typed meaning stays in the `Value` layer as extension traits (below).

### Evaluators

`MultisetEvaluator<O>` stays the core trait. Built-ins become `impl<O: OutcomeOrder> MultisetEvaluator<O>` (or
`O: OutcomeArith` for `sum`, straights and anything doing outcome arithmetic) with **typed finals**:
`largest_count -> Int`, `highest_outcome_and_count -> (O, Int)`, `all_counts -> Vec<Int>`, `expand -> Vec<O>`,
`keep -> Vec<O>`, comparisons `-> bool`, argsort/lexi `-> Vec<Int>` / `Int`.

The `Value` layer must still see what Python sees: Python builds the result die from the final outcomes *as Python
values*, merging equal ones with Python semantics. So the driver converts **per final state, before the die is
built**, never after:

```rust
/// Evaluates and builds the result die from `into(final)`, as Python builds it from final_outcome's values.
pub fn evaluate_into<O, E, P>(ev: &E, inputs: &[Input<O>], into: impl Fn(E::Final) -> P) -> Result<Die<P>>
where O: OutcomeOrder, E: MultisetEvaluator<O>, P: OutcomeOrder;

impl<O: OutcomeOrder, E: MultisetEvaluator<O>> E { fn evaluate(&self, ...) -> Result<Die<E::Final>> } // into = identity
// Value layer: evaluate_into(ev, inputs, IntoValue::into_value), IntoValue for Int, bool, (A, B), Vec<T>, Value.
```

- **Boilerplate (Q8):** the seven poker evaluators share one helper trait with defaults, blanket-implementing the
  evaluator. Each poker evaluator is then a state type plus two short methods:

  ```rust
  /// An evaluator that folds `(outcome, count)` into a state and reads the result off the final state: no sizes,
  /// no extra outcomes, any order unless `order` says otherwise.
  pub trait CountFold<O: OutcomeOrder> {
      type State: Clone + Eq + Hash + Default;
      type Final;
      fn order(&self) -> Order { Order::Any }
      fn fold(&self, state: &Self::State, outcome: &O, count: &Int) -> Result<Self::State>;
      fn finish(&self, state: &Self::State) -> Result<Final<Self::Final>>;
  }
  // C2: not a blanket impl (it would make every other generic `impl MultisetEvaluator<O>` a coherence error);
  // `count_fold_evaluators!(A, B<F>, ...)` writes the shared impl for each listed type (evaluator/fold.rs).
  ```

  The per-file `int`/`int_tuple`/`sorted_descending` helpers go; there is one `IntoValue`.
- **Tuple counts (`MultiDeal`)** are Python dynamism (Python's arithmetic simply runs on a tuple count). The core
  trait keeps `next_state_counts(&[Count])`, whose default passes int counts to `next_state` and answers
  `Error::Unported` for a tuple. `next_state_values` leaves the core trait: the `Value` layer has a trait
  `TupleCountEvaluator` (next_state over `Value` counts) and a wrapper `WithTupleCounts<E>` implementing
  `MultisetEvaluator<Value>`; the few evaluators Python lets run on tuples (keep, sum, argsort, lexi, expand...)
  implement it in `value/evaluators.rs`.
- Host (user) evaluators stay `MultisetEvaluator<Value>` through `host::HostEvaluator` (section 5).

### map, explode, Again, reroll, statistics

```rust
impl<O: OutcomeOrder> Die<O> {
    pub fn map<P: OutcomeOrder>(&self, f: impl FnMut(&O) -> P) -> Die<P>;              // infallible for Outcome P
    pub fn map_die<P: OutcomeOrder>(&self, f: impl FnMut(&O) -> Die<P>) -> Die<P>;
    /// The general form the Value layer uses: outcome, die, or Reroll; Break/Restart under `repeat`.
    pub fn try_map<P, E>(&self, f: impl FnMut(&O) -> Result<Mapped<P>, E>, repeat: Repeat) -> Result<Die<P>, E>;
    pub fn reroll(&self, which: impl Fn(&O) -> bool, depth: Depth) -> Die<O>;
    pub fn filter(&self, which: impl Fn(&O) -> bool, depth: Depth) -> Die<O>;
}
impl<O: OutcomeArith> Die<O> {
    pub fn explode(&self, which: impl Fn(&O) -> bool, depth: u32, end: Option<&O>) -> Result<Die<O>>;
}
pub enum Mapped<P> { Outcome(P), Die(Die<P>), Reroll, Break(Option<P>), Restart }
```

- `map_tools` (repeat, Markov chains, `mean_time_to_sum`) is generic over `O: OutcomeOrder` (`OutcomeArith` where it
  adds). Python's argument handling (`star` inference, several dice as cartesian arguments, `dict` repl,
  `time_limit` defaults) is facade work in the `Value` layer, which builds the closure.
- `Again<O>` (today `Again<F, L>`) is an expression enum generic over `O: OutcomeArith`, with `std::ops` on it so a
  Rust user writes `Again::new() + 1`; depth and count modes as ported.
- **Statistics, typed:** exact, generic over `O: Outcome + ToRatio` (`ToRatio` for the integer types and `Ratio`):
  `mean() -> Ratio`, `variance() -> Ratio`, `probability(..) -> Ratio`, `standard_deviation() -> f64`. This matches
  Python for `int` dice (Fraction results). The Python-typed versions (float vs Fraction by outcome type, `percent`,
  Python's `round`) are `value::PyStatistics`.
- **Extension traits for Python behaviour.** Methods that only make sense with Python values live on traits in
  `value/` (`PyDie`, `PyStatistics`, `PyFormat`, `PyPool`), implemented for `Die<Value>` etc. Rust resolves
  `die.mean()` to the inherent typed method for `Die<i64>` and to the trait for `Die<Value>` (inherent candidates
  with unmet bounds are skipped; checked with rustc). Associated functions without `self` must not share names.

### Formatting

`impl<O: FormatOutcome> Display for Die<O>` prints icepool's default `str(die)` (markdown table: outcome, quantity,
probability). The format-spec engine (`format.rs`: markdown, csv, bbcode, html, col specs) is generic over

```rust
pub trait FormatOutcome { fn fmt_outcome(&self) -> String; fn parts(&self) -> Option<Vec<String>> { None } }
```

implemented for the integer types, `bool`, `String`, `[T; N]` and tuples (`parts`: one column each, as Python
splits tuple outcomes), and for `Value` with Python's `str`. Probability text uses the exact `Ratio`, already
generic.

## 3. Errors: domain variants in core, exception names in the bindings

`Error::{Type, Value, Index, Key, ...}(String)` and `python_type()` (Q4) are replaced:

```rust
// error.rs
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    // Arguments
    InvalidArgument { argument: &'static str, message: String },   // a value outside what is allowed
    WrongType { message: String },                                // an argument or operand of the wrong kind
    IndexOutOfRange { message: String },
    MissingKey { key: String },
    // Outcomes (raised from the Value layer: Python's operators)
    Incomparable { op: &'static str, left: &'static str, right: &'static str },  // '<' not supported between...
    UnsupportedOperand { op: &'static str, left: &'static str, right: &'static str },
    Unhashable { type_name: &'static str },
    DivisionByZero { message: String },
    // Dice and evaluation
    EmptyDie { operation: &'static str },
    ConflictingOrder,
    UnsupportedOrder,
    // Limits
    Overflow { message: String },     // a result no machine integer holds (i64 shift, float conversion)
    TooLarge { message: String },     // a size no allocation can hold (memory.rs pre-checks)
    RecursionLimit { message: String },
    // Coverage
    NotImplementedUpstream { message: String }, // icepool itself raises NotImplementedError
    Unported { what: &'static str },            // Python answers, the port does not yet
    // Callbacks
    Callback(host::HostError),                  // raised by a host callback; the host re-raises it unchanged
}
impl std::error::Error for Error {}
impl Display for Error { /* the message alone, no "TypeError: " prefix; Python's own text where Python has one */ }
```

- **One variant, one Python exception, always.** If a domain condition raises different exception types at
  different Python sites, it is two variants. C4 starts from the list above, audits every site (176 helper calls,
  ~370 `Error::X(` sites), and adds or splits variants until the mapping is a total `match` with no message
  inspection. No stringly-typed matching anywhere.
- **Messages are kept verbatim** where they are Python's text: upstream tests and the facade may match on them.
- **The exception mapping lives in the bindings, not in core:** `icepool-py/src/errors.rs` maps each variant to a
  PyO3 exception type (`ConflictingOrder` to icepool's `ConflictingOrderError`, `Callback` re-raises the original
  `PyErr`); the harness runner has its own `fn python_exception(&Error) -> &'static str` for wire results (fixtures
  only check that both sides raised; the name is informational). icepool-wasm maps variants to JS error classes
  (section 6). The runner-only emulations of Python builtins (`StopIteration`, unpack lengths, `dict` updates) are the
  runner's own error type (`PErr`), never core variants.
- `Error::Unported` is reported by the runner as `unimplemented: ...` (never a match), as today.
- Typed-API conventions: programmer errors that Python raises as `ValueError` on arguments the caller controls
  (`d(0)`, negative keep counts) **panic with Python's message** in the typed API (like `slice::chunks(0)`), and
  return `Err` in the `Value` layer. Everything the caller cannot cheaply pre-check (evaluation order conflicts,
  callback errors, `TooLarge` sizes) returns `Result`.

## 4. Public Rust API

What docs.rs shows for `icepool-core` (crate root), everything else `pub(crate)`:

```rust
pub use die::{Die, Depth};                pub use function::{d, z, pointwise_max, pointwise_min};
pub use pool::Pool;                       pub use deck::{Deck, Deal, MultiDeal};
pub use again::Again;                     pub use outcome::{Outcome, OutcomeOrder, OutcomeArith, ToRatio, FormatOutcome};
pub use order::Order;                     pub use error::{Error, Result};
pub use int::Int;                         pub type Quantity = dashu_int::UBig;  pub type Ratio = dashu_ratio::RBig;
pub mod evaluator;   // MultisetEvaluator, CountFold, the built-in evaluators, Final, evaluate_into
pub mod expression;  // multiset expressions (keep, unique, union, ...) and multiset_function
pub mod value;       // Value: the dynamic outcome with Python's semantics, for language bindings
pub mod host;        // host-callback traits, for language bindings (section 5)
```

`program`, `pysort`, `khepri`, `trig`/`libm` leftovers, `creation_args::DieArg`, `ownership`, `memory`, `interrupt`
internals are not public. A binding that needs something gets a deliberate public API in `value` or `host`
(documented as "for language bindings"), never a widened `pub mod`.

The typed surface:

```rust
use icepool_core::{d, Die};

let d6: Die<i64> = d(6);                         // panics "sides must be at least 1." for d(0), as Python raises
let two = &d6 + &d6;                             // std::ops on Die and &Die, and Die op O
let shifted = &d6 - 1;
let best3 = d6.pool(4).highest(3).sum();         // Pool<i64> -> Die<i64>
let hits: Die<bool> = d(20).ge(11);              // comparisons are methods: lt le gt ge equal not_equal
let damage = hits.map(|&h| if h { 6 } else { 0 });
let three_d6 = d6.sum_rolls(3);                  // Python's 3 @ d6
let mean = two.mean();                           // Ratio, exact: 7
println!("{two}");                               // icepool's markdown table
for (outcome, quantity) in two.items() { /* ascending */ }

Die::from_iter([1, 2, 2, 3]);                    // one each, merged: 2 has quantity 2
Die::from_weights([(1, 2u32), (2, 1)]);          // (outcome, quantity) pairs
Die::from(1..=6);                                // ranges
Die::single(5);                                  // Die([5])
```

- `std::ops`: `Add Sub Mul Div Rem Neg Not BitAnd BitOr BitXor Shl Shr` for `Die<O>` where `O` has the operator,
  on owned and borrowed dice and with an `O` on the right; `i64 + Die<i64>` etc. by macro for the primitive integers
  (orphan rule). Overflow behaves as `O`'s own operator does in debug builds: it panics (checked ops, message
  "attempt to add with overflow"); `Die::try_binary(&self, other, f)` is the checked, fallible form.
- Comparisons that return dice are methods (`lt le gt ge equal not_equal`; `eq`/`ne` stay `PartialEq`'s, which is
  Python's `die.equals`). Python operators without a Rust one get descriptive names: `@` is `sum_rolls`, `//` is
  `floor_div`, `**` is `pow`.
- Iterators: `items()`, `outcomes()`, `quantities()` return `ExactSizeIterator`s; `IntoIterator for &Die`.
- `Display` for `Die`, `Pool`, `Deck`, `Error`; `Debug` everywhere.
- Docs: `#![warn(missing_docs)]`, a crate-level guide (section 7), and a doctest on every public item.

## 5. Crates, host callbacks, feature flags

```text
crates/
  icepool-core/     the library. Deps: dashu-*, rustc-hash. No serde_json, no runner, no binary.
  icepool-py/       PyO3 extension `icepool_rs` behind python/icepool. Depends on icepool-core only.
  icepool-wasm/     wasm-bindgen exports + the TypeScript package in js/. Depends on icepool-core only.
  icepool-khepri/   khepri's f2f engine (application code, typed Die<i64>/[i64; 4]); used by icepool-wasm's
                    `calculate` (feature `khepri`) and by the runner's `khepri` op.
  icepool-harness/  test-only: the JSON program runner (today's program/), the `icepool-run` binary, and (once
                    moved) the fixtures test. Depends on icepool-core, icepool-khepri, serde_json. Never a
                    dependency of a production crate.
```

- **The runner is a client of the public API.** It uses `icepool_core::{..., value, host}` like the bindings do.
  Moving it out is what proves the public API complete.
- **Interim, until the protected `crates/icepool-core/tests/fixtures.rs` moves** (it calls
  `icepool_core::program::run`; QUESTIONS.md asks the loop owner to move it to
  `crates/icepool-harness/tests/fixtures.rs` calling `icepool_harness::run`): `program/` stays in core behind a
  non-default feature **`runner`** (serde_json an optional dependency of it). icepool-harness depends on
  `icepool-core = { features = ["runner"] }`, so `cargo build/test --workspace` (the gate) unifies the feature on
  and the fixtures test compiles; maturin and wasm-pack build their crate alone, without it. The `icepool-run`
  binary moves to icepool-harness at once (same name, so `$CARGO_TARGET_DIR/release/icepool-run`, which
  harness/core/tool.py runs, is unchanged). C5 verifies the unification claim before relying on it.
- **Host callbacks in one module, `icepool_core::host`:** `HostObject` + `HostRef` + `HostError` (value/host.rs),
  `HostSource` (host_source.rs `HostSourceObject`), `HostOperator` (operator/mod.rs), `HostEvaluator` and
  `HostFunction` (program/host.rs). Their arguments and results are core types only: what crosses a host boundary is
  `host::HostValue = Value | Die<Value> | Pool<Value> | Deck<Value> | Again<Value> | Mapped<Value>`; the runner's
  `PValue`/`PErr` wrap it, they are not part of it.
- **Feature flags:** `icepool-core/runner` (interim, removed when the runner moves), `icepool-wasm/khepri`
  (`calculate`), `icepool-wasm/runner` (the `run()` export that `run-wasm.sh` drives in the long gate:
  `icepool-core/runner` now, an optional dependency on icepool-harness after C5b). None default. No feature for
  `Value`: it is part of the core (both bindings
  and the runner need it) but no generic code names it (section 1).
- **Test-only code** sits in `#[cfg(test)]`, `[dev-dependencies]`, `tests/`, or the harness crate. The Python
  package's tests (`crates/icepool-py/tests`), run-facade scripts and the JS tests (`js/test`) are outside the
  shipped file lists (section 8).

## 6. The JavaScript / TypeScript API

Today's JS package is classes over a JSON-op RPC (`call(op, args)`, `evaluate`, `fromWire`, handles in a JSON wire
format; Q5). Decision: **a TypeScript API written in TypeScript (`js/src/*.ts`, compiled by tsc; the `.d.ts` is
generated, not hand-written), over typed wasm-bindgen exports of the core** (`#[wasm_bindgen] struct WasmDie`,
`WasmPool`, ... in `icepool-wasm/src/api/`, taking and returning `JsValue`s converted by
`icepool-wasm/src/convert.rs` straight to `Value`, no JSON). The program runner is not in the package.

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

The API mapping (all of `icepool/__init__.py` is covered the same way; this is the pattern, C6 does every row):

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

- **Evaluator states in JS** must be hashable the Python way: primitives, `Fraction`, `Float`, and (nested)
  readonly arrays of them, converted to `Value` once per call. Anything else is a `TypeError`-kind `IcepoolError`
  ("unhashable"), as Python raises for a list state. This is the one genuine JS constraint (ANALYSIS.md).
- **Callbacks** (`map`, `reroll(which)`, `groupBy`, evaluators) are plain JS functions, called synchronously from
  the core through `host::HostFunction` / `HostEvaluator`; re-entrant calls into the API from a callback work.
- **No `any` in the public types.** `Outcome` is a union type; `Die<T extends Outcome = Outcome>` is generic so
  `d(6)` is `Die<number>` in the types.
- The JS fixture suite keeps running every fixture, through a **test-only adapter** in `js/test/` that translates
  fixture programs into calls of this public API.

## 7. Documentation

- **Rust:** rustdoc. The crate-level guide is `crates/icepool-core/README.md` included by
  `#![doc = include_str!("../README.md")]`: first dice, operators, pools and keeping, evaluators (built-in and a
  hand-written `CountFold`), map/explode/Again, decks, exact quantities, typed vs `Value`, errors. Every public item
  has a doc comment with an example; examples are doctests and run with `cargo test`. `cargo doc --no-deps` must
  build without warnings (`RUSTDOCFLAGS=-D warnings`, in CI, C8). Provenance comments keep citing the Python file
  and function (no line numbers).
- **Python:** upstream's documentation is the documentation: `python/icepool` keeps upstream's docstrings
  verbatim, so upstream's rendered docs apply. `python/DIFFERENCES.md` lists what differs on the Rust backend,
  generated from DIVERGENCES.md's user-visible entries plus the by-design items (per-call caches, the
  `ICEPOOL_BACKEND` switch, `NotImplementedError("icepool_rs: ...")` for unported cases, recursion limits).
- **JS/TS:** its own docs in `crates/icepool-wasm/js/docs/`: a guide (install, first dice, pools and evaluators,
  decks, Again/explode, custom evaluators in JS, BigInt quantities, disposing, differences from Python icepool), TSDoc
  with an example on every exported symbol, an API reference generated by TypeDoc (`npm run docs`), and the package
  README. Every fenced `ts` example in the guide and the TSDoc is executed by `js/test/doc-examples.test.mjs`.
  typescript and typedoc are devDependencies of the JS package only.

## 8. Production builds contain no test code

What ships: the Python wheel (maturin, `python-source = "../../python"`: the `icepool` facade package plus the
`icepool_rs` extension; `ICEPOOL_BACKEND` defaults to `rust` in the wheel only, via a build-time constant, so the
harness keeps choosing explicitly) and the npm package (`npm pack` of `crates/icepool-wasm/js`). Neither contains
tests, fixtures, the runner, harness code, catalogue functions, or JSON-program code.

`scripts/check-production.sh` (the gate runs it once it exists; it must stay under a minute when builds are fresh):

1. **Dependency graph:** `cargo tree -p icepool-py -e normal` and `cargo tree -p icepool-wasm -e normal
   --target wasm32-unknown-unknown` must not contain `icepool-harness` or `serde_json`, and must show icepool-core
   without the `runner` feature (`cargo tree -e features`).
2. **Wheel contents:** `maturin build --release -m crates/icepool-py/Cargo.toml`, then `unzip -l` the wheel: fail on
   any path matching `test|tests|conftest|fixtures|harness|catalogue|run-facade|program`.
3. **npm contents:** `npm pack --dry-run --json` in js/: fail on `test/`, `*.test.*`, `run-wasm*`, `catalogue*`,
   `fixtures*`, `bench*`.
4. **Binary contents:** the extension `.so` and the `.wasm` must not contain the strings `icepool-harness`,
   `icepool_harness`, `src/program/`, `serde_json` (panic locations embed source paths, so any linked runner code
   shows); the wasm module's exports (read with node's `WebAssembly.Module.exports`) must not include `run`, `call`,
   `evaluate`, `from_wire`, `wire_result`.
5. Prints what it checked (file counts, sizes), exits non-zero with the offending paths/strings on failure.

Tools it needs (maturin, wasm-pack, node, npm) are already used by the long gate; if one is missing the script
says so and fails, rather than passing silently.

## 9. Migration order (C1-C8), gate green at every step

The runner keeps driving the oracle comparison throughout: it runs on `Value`, so every step that keeps the
`Value` instantiation's behaviour keeps every fixture passing. Each bullet is a commit-sized step.

1. **C1 typed outcomes.** (a) Add `OutcomeOrder`/`Outcome`/`OutcomeArith` beside the old trait; `impl OutcomeOrder
   for Value` delegates to the existing `py_*`. (b) Switch generic code to `OutcomeOrder` (`try_*`), module by
   module. (c) Move `pysort.rs` to `value/sort.rs` and the `py_*` methods into `value/`; delete the old trait and
   `SumOutcome`. (d) Typed methods on `impl<O: Outcome> Die<O>` return plain values; khepri (still in core) compiles
   against them.
2. **C2 generic features.** Evaluators file by file (`CountFold` first with poker, then basic, keep, lexi,
   argsort, comparison) with `evaluate_into` + `IntoValue` in the `Value` layer; then map/map_tools, explode/Again,
   to_pool/expand, statistics (`ToRatio`) and formatting (`FormatOutcome`), Python-only behaviour into `value::Py*`
   traits. Move khepri into `crates/icepool-khepri` (the runner's `khepri` op and wasm `calculate` depend on it).
3. **C3 public API.** `std::ops`, typed constructors, `Display`, iterators, the curated root of section 4,
   everything else `pub(crate)`; icepool-py and icepool-wasm switch to the public API (`value`, `host`, root),
   which grows deliberately where they need it. The runner, still in core, keeps `pub(crate)` access. Crate guide,
   doctests, `missing_docs`.
4. **C4 errors.** Section 3: variants, audit, mapping in icepool-py and in the runner; delete `python_type()`.
   The upstream suite (exception types) and the fixtures (raise vs not) check it.
5. **C5 crates and production builds.** (a) `crates/icepool-harness` with the `icepool-run` binary; core's
   `program/` behind `runner`, serde_json optional; verify `cargo test --workspace` still runs the fixtures test.
   (b) `host` module consolidation. (c) icepool-wasm: `run()` behind its `runner` feature (run-wasm.sh enables it),
   `calculate` behind `khepri`. (d) `scripts/check-production.sh`. (e) QUESTIONS.md: the fixtures.rs move.
   **C5b** (new task, after the owner moves fixtures.rs): move `program/` into icepool-harness as a client of the
   public API; drop the `runner` feature.
6. **C6 TypeScript API** (section 6), with the fixture adapter and API tests mirroring upstream's Python tests;
   then **C6b** its documentation (section 7).
7. **C7 Rust tests**: typed `Die<i64>` unit tests and doctests; property tests comparing each typed path with the
   `Value` path on the same integer dice (they must agree exactly), runnable with plain `cargo test`.
8. **C8 presentation**: README, MODULE-MAP, CI, split the oversized files (`program/ops.rs`,
   `program/expression.rs`, `value/ops.rs`) by domain.

Never in one step: changing behaviour and moving code. A move is a move; a semantic change has fixtures.
