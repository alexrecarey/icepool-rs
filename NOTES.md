# Notes for the next iteration

Gotchas, decisions and layout notes, written by the iterations themselves. Keep it current: fix or delete what
turns out wrong. Newest decisions at the top of each section.

## Decisions

- **Cycles and the collector (R4).** The review's "leak" (20k x `d(6).map(lambda o, big=big: o)` +
  `pool(3).sum()`, maxrss +1.7 MB) was warm-up: after 500 warm-up iterations 20k more add 0 KB of maxrss and a few KB
  of traced memory (icepool's caches) on both backends. The real leak was cycles: a Python object held by the core
  (a host outcome `Value::Host(PyHost)`, a user's source `PyHostSource`) that refers back to its die was never
  collected. `icepool_core::ownership`: `Traverse` (impl beside each type: Value, Vector, Symbols, Die with its
  caches, Pool, the sources, Deck, Deal, MultiDeal, CompoundKeep, Weightless, HostSource) lists each `Arc` clone a
  value holds; `owned_hosts(root)` keeps the host objects for which every `Arc` on the way has all its strong
  references held from inside `root` (Kahn over the Arc DAG, counting edges from owned nodes against
  `strong_count`). Each icepool_rs pyclass's `__traverse__` (`convert::traverse`) reports those. **Never report
  anything shared**: Python subtracts one per report from the object's refcount, and two reports for the one
  reference an Arc holds can free a live object. Under-reporting only leaks. A new pyclass holding core values
  needs a `__traverse__`; a new core type holding Values (or a new cache in DieData) needs its `Traverse` impl
  listing *every* Arc clone exactly once (a missing one is safe, a duplicate is not). No `__clear__`: the classes
  are frozen, and PyO3's guide notes immutable references need none. Concurrency is safe: a clone held elsewhere
  (another thread's computation, a callback's stack) raises `strong_count`, so nothing below it counts as owned.
  `RsDeal` / `RsMultiDeal` hold their deck's `Py<RsDeck>` (reported) plus the keep tuple / hand groups, and build
  the core value per call (`Deal::from_parts`, `MultiDeal::from_parts`): with a second clone of the cards neither
  wrapper could report them. So do (R4b) `RsCompoundKeep` (its inners' `Py<RsPool>` / `Py<RsDeal>` /
  `Py<RsCompoundKeep>`, or for a user's generator its source; the facade passes `_rust_inner(inner)`) and `RsPool`
  (each `Pool(dice, times)` argument's `Py<RsDie>` with its count, zero counts included, a merged pool all its pools'
  in turn; `Pool::from_parts(dice, keep_tuple)` = `Pool::new` with that keep tuple; `dice()` hands back the held
  RsDie objects via `Die::ptr_eq`). Keeping and multiplying counts touch only the keep tuple; the rest remakes the
  core value per call (cheap: cloning Arcs, merging and sorting the dice). A new pyclass deriving from another's
  core value should do the same. Still not collected: cycles through two objects that share core data
  *non-structurally*: a host outcome's `Arc<PyHost>` holds one Python reference, so two live dice sharing it (a die
  and its popped die cached by the facade's `_popped_min`; a pool or deal and a source made from it) can't report
  it. Fixing that would mean one Python reference per holder (a new `PyHost` per clone), a core design change. Dice
  made by a callback (`map`) get new PyHosts and are fine. Tests: crates/icepool-py/tests/test_leaks.py (cycles
  per type, a shared outcome survives, a collection at every allocation changes no result, the review's loop
  stays flat). The upstream suite also passes with `gc.set_threshold(10, 2, 2)` (1716). JS: `Handle.release()` /
  `[Symbol.dispose]` (icepool.d.ts references lib esnext.disposable); `handleOf` throws a TypeError for a released
  handle; `Again.release()` does nothing. A JS callback the core holds sits in core.js's global `functions` map
  until the core drops it, so a callback referring to its own handle is never collected without `release()`.
  Tests in js/test/api.test.mjs. Their WeakRef check must not `deref()` before `gc()`: deref keeps the target
  alive to the end of the job.

- **Huge sizes and panics (R2).** `memory.rs`: every allocation sized by user input goes through `check_len` /
  `try_with_capacity` / `try_repeat` / `try_collect` (refused past `MAX_ALLOC_BYTES`: 64 GiB native, 256 MiB on
  wasm32, then `try_reserve_exact`) or, for a big-int power, `check_pow`; both give the new `Error::Memory`
  (Python's `MemoryError`; PyMemoryError in the facade). Sites: `d` / `z`, `die_sequence` (the `rolls` copies),
  `cartesian_product_items` / program `iter_cartesian_product` (`expand::product_size` up front: the result grows
  one item at a time and would abort when growing fails), `d_pool`'s `[k] * v`, `one_hot_items`' `[False] * len`,
  explode_to_pool's depth, the evaluation driver's joint pops (`len * len`), `str * n` / `tuple * n` (only bites
  on wasm32, under `MAX_REPEAT`), `highest/lowest_single`, finite `reroll` depth, `matmul_terms`. argsort's
  `limit` pad goes through `list_len`. `MAX_INT_BITS` is 2**30 on wasm32. The audit of the other
  `with_capacity` / `vec![x; n]` / `repeat_n` sites: they are sized by data that already exists, by `list_len`, or
  by the pool / deal size limits. Not covered: results that grow step by step from huge *work* (`d(10**6) *
  d(10**6)`). `Pool::denominator` (`den ** count`, up to 2**24 dice) is fallible since R2b: `memory::check_bits`
  on `sum(bits(den) * count)`, MemoryError past the cap (on wasm32 from 129-bit denominators in a full pool). `die.highest/lowest(rolls)` lost its 2**16-rolls OverflowError (Python answers `d1` for any
  rolls); `check_pow` skips bases 0 and 1. Fixtures: fixtures/r2 (huge-sizes, large-but-answered).
- **Panics.** icepool-run: `catch_unwind` (as before). Python: PyO3 wraps every `#[pyfunction]` / `#[pymethods]`
  in `catch_unwind` and raises `pyo3_runtime.PanicException` (a BaseException; exported as
  `icepool_rs.PanicException`). A Python callback raising PanicException makes PyO3 *resume* the panic in Rust,
  which is how crates/icepool-py/tests `test_a_panic_raises_panic_exception_and_leaves_no_state` gets a real
  unwinding panic through the core. Unwinding gives back `recursion::Level` / `Frames` and now `VARS` too
  (`multiset_function::VarsFrame`, popped on drop). WASM (panic = trap, no unwinding, no destructors): the panic
  hook (`#[wasm_bindgen(start)]`) keeps the message; js/core.js `wasm()` wraps every `W.*` call, `callback`
  rethrows a `WebAssembly.RuntimeError` instead of handing it to the core, and once the outermost call is left it
  restores the shadow stack pointer (`__stack_pointer`, exported by crates/icepool-wasm/build.rs, reached through
  `stack_pointer()` = `wasm_bindgen::exports()`: without this every trap leaks the abandoned frames' stack and a
  few hundred traps hit "memory access out of bounds"), calls `recover()` (`program::reset_after_trap`: VARS
  cleared, recursion levels 0; returns the hook's message) and throws `IcepoolError("PanicException")`. Handles
  stay valid (HANDLES is only borrowed around code that cannot panic; `release` drops the value after the
  borrow). JS functions an abandoned call registered leak. A trap inside a die's `OnceLock` initialisation would
  leave it running (each later use of that die panics again: an error, not a crash). `run` / `calculate` (R2b):
  core.js exports wrapped versions (`run(programJson)`, `calculate(engineInput)`, typed in icepool.d.ts);
  run-wasm.mjs and test-calculate.mjs `bind` core.js to their wasm-pack build and call those, so the protocol
  exercises the recovery (it no longer reloads the instance). khepri's worker should import `calculate` from the
  package rather than call the raw export. The JS
  API test simulates a trap by throwing `WebAssembly.RuntimeError` from a callback (no reachable panic is left to
  trigger one); a deliberate `assert!` in `d()` was used once by hand to check the hook's message reaches JS.

- **Ctrl-C and the GIL (R3).** `interrupt.rs`: `poll()` asks the host's check (`set_check`, set once by the Python
  facade: `convert::check_signals` = `Python::attach(py.check_signals())`, so a `KeyboardInterrupt` comes back as
  `Error::Host` and is re-raised unchanged) at most every 50 ms per thread (`Instant`; without a check it returns
  before reading the clock, so the native runner and WASM never touch time). Polled per pop in
  `evaluate_backward`/`forward`, per row of `Die::binary_operator` (so sums, `@`, `_sum_all`), per die argument of
  `merge_weights_lcm` / `merge_duplicates` (now fallible), per outcome of `step_transition_die(_with_time)`, per
  state of the Markov BFS and per elimination row, per pool pop product, per cartesian product item, per
  `mean_time_to_sum` target. Put new polls at a loop level whose iterations do real work (not per hash insert).
  Any error escapes, so a new `match`/`.ok()` that swallows *all* errors in a polled loop would swallow Ctrl-C:
  catch specific kinds only. GIL released (`py.detach`) for built-in evaluations (`RunBuiltin::visit`; the
  `BuiltinVisitor` evaluators are `Sync`), `matmul_core`, and `binary_operator` with a built-in operator on dice
  without host outcomes (`Binary::native`; an `Unsupported` result, `str %` with specifiers, runs it again with
  the GIL and Python's operator). Host callbacks already take the GIL themselves (`Python::attach`), so a host
  outcome inside a detached computation is fine, just slower. Callback-driven paths (map, host evaluators,
  map(repeat)) keep the GIL: Python checks signals in the callbacks itself, and the polls cost little with the GIL
  held. After an interrupt the evaluation's memo is dropped before the exception reaches Python (0.5 s for
  `d(30).pool(60).expand()`). Tests: crates/icepool-py/tests/test_interrupt.py (not in the gate; ~10 s).

- **A deck beside a multiset expression, and mx_str of a non-expression (M202610160934a).** `deck <op> expr` is
  `Deck.__op__`, which takes any iterable: `Counter(expr)` iterates the expression by `__getitem__` (`expr[0]`,
  `expr[1]`, ...: `keep(i)` evaluated into a die) until an index raises IndexError, so the keys are dice. No die is
  among the deck's cards, so `+ | ^` keep each die's count of 1, `- &` drop it, and `Deck(+data)` flattens the
  dice that are left (their quantities as card counts) after the deck's own cards. Port: `program/deck.rs`
  `deck_op_expression` (from `deck_op` when the other operand is an expression; `expression::binary_op` sends a
  deck on the left there). Inside a multiset function `expr[i]` is a raw result and never raises, so Python never
  stops: the port says Unimplemented. Likewise an expression evaluated with the index (an operator, a pool with negative
  keeps) and no outcomes at all is the empty die at every index; a keep generator (or a mixture of them) checks its
  keep tuple first, so it does stop. `mx_str` of a non-expression is plain `str()`: a value (`py_str`), a deck
  (`deck_str`), a die; an `AgainExpression` (`Again <op> expr`, either side) has no `__str__`, so its str is an
  address: `address("AgainExpression")`, Unimplemented, never generated (fixtures can't hold an address).
  Generators: deck.py `_other` now and then gives a small expression; `_gen_mx_str` now and then a deck on the left
  of `mx_binary`. Fixtures: fixtures/m202610160934a (edge-cases 266, generated 247, empty-dice 10).

- **str() of multiset expressions (T1c).** Node `mx_str` (multiset.py, kind "value"): `str(expr)` of a multiset
  expression, a MultiDeal, or the die a pool's int index evaluates to. Only `str`: every expression's `repr` (and the
  `str` of classes without a `__str__`: `weightless()`, map_counts, sort_pair / max_pair (not `==`) / versus,
  force_order (not Any), MultisetTupleIndex / Slice) is `object.__repr__` with an address. The port raises
  `Unimplemented` for those (`program/expression.rs` `address`); `gen_str_expression` never builds them. Core:
  `Operator::py_str(children, keep_index)` (operator/mod.rs) plus `BinaryOperator::symbol` /
  `KeepCountsComparison::symbol`. `MultisetKeep.__str__` shows the index *as given* (`[1, Ellipsis]` vs `(1,)` vs
  `slice(1, None, 1)` vs `1.5` vs a die's table), which the core's `MultisetKeepIndex` does not keep: `OpNode` carries
  it (`keep_index: Option<KeepIndexArg>`); `highest` / `lowest` / `middle` give `slice(a, b, None)`. Pools print from
  `dice()` (hash_key order, zero-count dice dropped, as Python's `_dice`); `CompoundKeep([...])` prints its inners
  from their *sources* (a Pool/Deal/CompoundKeep source keeps the dice/deck and keep tuple). Parameters carry their
  name (`MExpr::Param { name }`): `FunctionDef::positional_name` copies `_get_positional_name` with `mf_def`'s
  parameter names `p0..` / `rest` (a JS host function gets the same names; str is not exposed there). Known gap,
  never generated: `expr * True` stores `1` (the core's constants are `i64`), so its str would say `* 1` where
  Python says `* True`. Node `break_str` (maps.py): `(repr(b), str(b), hash(b) == hash(b))` for `b = Break(x)`
  (hash raises for a VectorWithTruthOnly outcome). `vector_method` repr/str now also gets VectorWithTruthOnly.
  `@multiset_function` bodies now and then return `mx_str` of an expression over the variables (a str constant).
  Not reachable through the public API (left uncovered on purpose): `Counts.__str__` / `__repr__` / `__eq__` /
  `__hash__` (only `die._data` is a Counts; keys()/values()/items() are views), `SparseVector.__str__` /
  `__delitem__` (internal to absorbing_markov_chain_impl, never printed or deleted from). Fixtures: fixtures/t1c
  (str-edge-cases 124, str-generated 441).

- **The facade's leftover Python work (S5c).** Core: `Die<Value>::append` / `remove` / `one_hot_items`
  (population.rs; they return the `dict` items for `Die(...)` / `Deck(...)`, like `pad_to_denominator`),
  `Deck::zero` / `split`, `Die<Value>::mean_time_to_sum(cache, target)` (die.rs: extends the caller's cache as
  Python's loop does; every recursive call hits the cache, so non-int outcomes / targets are the list-index or
  `range()` TypeError), `expand::cartesian_product_items` (the overwrite-semantics `data` dict, shared by Die and
  Deck results). Program layer: program/population.rs; `collection::cartesian_product` now takes decks (mixed with
  dice: TypeError, checked argument by argument with Reroll) and multiset expressions (their `expand()` items);
  `x in tuple` (map.rs `contains`) compares with Python's `==` like the list branch (`Symbols('') == ()`).
  Nodes (harness/grammar/population.py, kind "value" unless noted): `pop_append`, `pop_remove`, `to_one_hot`,
  `deck_zero` (kind deck), `deck_split`, `mean_time_to_sum` (a tuple of results for a list of targets, one cache),
  `cartesian_product` (`outcome_type` tuple or Vector), `iter_cartesian_product` (a tuple of `(outcomes, q)`).
  The fuzzer picks root kinds uniformly, so "value" nodes get few programs: generate them directly (a throwaway
  `fixtures make tmp-x/...` of `NODES[op].gen` output, `fixtures verify` both ways, then move the file out of
  fixtures/). At S5c ~46k such programs plus 5k-program fuzz runs clean both ways; fixtures/s5c (edge-cases 399,
  contains-tuple-eq, largest-count-wild-lazy-truth).
- Facade (crates/icepool-py/src/population.rs): `population_append` / `population_remove` (then
  `_from_rust_data`), `to_one_hot(die, own_outcomes, outcomes, deck)` (a given sequence's `len` / `in` / `index`
  are Python's, called from Rust; `len` only once there is an outcome), `deck_zero` / `deck_split`,
  `call_per_outcome` (Deck.map's calls; `Deck(...)` of the results stays upstream's), `mean_time_to_sum(die,
  cache_list, target)` (appends to the die's own `_mean_time_to_sum_cache`), `iter_cartesian_product(items...)`
  and `cartesian_product_data(outcome_type, items...)` (a dict; `outcome_type` called back unless it is `tuple`).
- **Built-in evaluators over tuple counts (S5d).** `MultisetEvaluator` has two new hooks: `next_state_values`
  (called by the default `next_state_counts` when some count is a `Count::Tuple`, every count as its Python value;
  default `Unsupported`) and `reads_sizes` (default `true`; `false` makes a tuple size reach `initial_state` /
  `final_outcome` as `None`, for evaluators that never read sizes). Python has no special case, so each built-in
  ports its `next_state` over `Value`s: sum / mapped sum (`count == 1` False, `outcome * count` repeats, `+`
  concatenates), size (`0 + tuple` TypeError), product (`1 * tuple`, then tuple * tuple TypeError), empty (`== 0`
  False), expand (`(outcome,) * tuple` TypeError), comparisons (`<` between tuples lexicographic, with an int
  TypeError, `==` False; disjoint's `left > 0 and ...` short-circuits), count_subset (`right == 0` False, `//`
  TypeError), largest_count / largest_count_and_outcome / all_counts / wild largest_count (Python's `max` / `sorted`
  / `+` on Value states; wild's state is `Option<(Value, Value)>` now). Those whose `next_state` starts by comparing
  the count with an int (keep, highest_outcome_and_count, the straights) return `tuple_compared_with_int`; keep
  overrides `initial_state_counts` (a tuple size is read only to count from the other end: TypeError). States
  produced from tuple counts can meet `int` counts later (a filter gives 0 for filtered-out outcomes), so the
  state-producing evaluators run their `int` path through the `Value` one. argsort and lexi: S5e (below).
- Facade: `evaluate` and `rust_call_tree` no longer refuse built-ins for tuple sources, and built-ins fed tuple
  counts by a host operator are no longer NotImplementedError. Since S5e `builtin_spec` names every built-in
  whatever the sources (the `tuple_counts` argument and `_tuple_source` are gone).
- **argsort / lexi over tuple counts (S5e).** `ArgsortEvaluator::step(state, Vec<Result<Int>>)` is the one
  `next_state`: a tuple count is the `Err` Python's arithmetic raises when it reaches it (drop: only below the
  limit, all counts compared; left / right: in turn, so a return at the limit before the tuple succeeds; grouped:
  below the limit always raises, `'>'` or `(i,) * tuple` by whether a positive int came first). Lexi:
  `next_state_values` always raises (`max(left, 0)`), `initial_state_counts` reads tuple sizes only in reverse
  order (None size: UnsupportedOrder first). All of them `reads_sizes() == false`. So with tuple counts the only
  successful results are argsort ones that reach the limit (limit <= 0, or left / right filling it at the one
  outcome); everything else is TypeError. builtin_evaluate generates the four argsort classes (1-3 inputs, ctor
  kwargs order / limit / limit_pad) and LexiComparisonEvaluator (evaluate kwargs sort_order ±1, lexi_tuple a
  list of 5). Fuzzed by a direct generator script (as S5d): 20.5k natively, 4k + the edge cases through
  run-facade.sh, clean. fixtures/s5e (edge-cases 330, generated 350); facade cases (S5e block).
- Node `builtin_evaluate` (multiset_function.py): `getattr(icepool.evaluator, cls)(*params, **ctor_kwargs)
  .evaluate(*args, **kwargs)` for the 23 built-in classes (`_BUILTIN_EVALUATORS`: arity and parameter
  generators), args mostly MultiDeals / `tuple_count_expression`s, now and then a pool. Function bodies
  (`_evaluation`) give whole tuple variables (or operators over them) to mx_evaluate / issubset / count_subset /
  keep 20% of the time. Fuzzed by calling generators directly (throwaway script: `G.Gen(rng, G.ported_ops(),
  depth)`, `G.NODES[op].gen(g, depth)`, `tool.oracle` vs `tool.Port`; needs `G.load_grammar()` first): at S5d 25k
  builtin_evaluate + 16k mf_call/mf_call_kw + 6k tuple_op/multi_deal/mx evaluations natively, 4k builtin_evaluate
  and 3k mf_call through run-facade.sh: clean. fixtures/s5d/edge-cases (258), s5d/function-edge-cases (46), also
  clean through the JS fixture runner; facade cases (S5d block).

- **Host sources and host operators (S5c).** The core driver now takes what it has no variant for:
  `MultisetSource::Host` (core host_source.rs: `HostSourceObject` callbacks, outcomes asked once, equality /
  hash the host's) and `Operator::Host` + `Statelet::Host(Value)` (a `HostOperator`'s `initial_state(order,
  outcomes, child_sizes)` / `next_state(state, order, outcome, child_counts)`; `Operator::initial_state` and
  `expression::initial_state` take the evaluation's outcomes for it). `MultisetSource::is_resolvable` /
  `order_preference` (and CompoundKeep's, Weightless's) return `Result` now. icepool-py host_source.rs: a facade
  source without `_rs` is a host source (`core_source` falls back to it; popped sources holding `_rs` are the
  core's again; `source_object` gives the Python object back); `UnsupportedOrder` from `pop` / `size` is the retry
  signal, from `outcomes` / `is_resolvable` / `order_preference` it propagates (outside Python's `try`).
  expression.rs `HostOperatorPy` for the spec `('host', dungeonlet.next_state, questlet.initial_state)`: called
  with a list of child counts, an `Order`, an empty source iterator and `()` arg counts. evaluator_rust.py: any
  free variable from a generator questlet is accepted (`_core_source`), sources passed through `_rust_sources`
  (compound_keep.py `_rust_source`: `_rs` or the object), an operator without a spec (user subclass, subclass of a
  built-in, or a cached dungeonlet disagreeing with its questlet) becomes the host spec, built-ins only without
  tuple sources (`_tuple_source`: any `MultisetTupleSource`). multiset_evaluator_base.py raises
  `NotImplementedError` for anything still not describable (custom Dungeon / Dungeonlet classes) instead of
  running upstream's driver. Gaps: counts / sizes from a user source or operator that are not ints / int tuples
  within 64 bits are NotImplementedError (built-ins fed tuple counts by a host operator run natively since S5d). Cases in crates/icepool-py/tests (user `Uniform` generator, `DoubleCounts` operator, `MyUnion(
  MultisetUnion)`): no grammar nodes, the program language has no user classes. Make test classes' `hash_key`
  include `type(self)`: the oracle's evaluators cache rooms by source equality across cases.
- Fixed in passing: `LargestCountWithWildEvaluator` (poker.rs) took every selector's truth value at once; Python
  keeps `is_wild or select(outcome)` as an object and asks its truth only in `if is_wild_early/late` (a Vector
  from `identity` raised only in the port). Selectors (program evaluate.rs `make_select_function`) return values.

- **Operators over tuple counts, operator equality, @multiset_function kwargs (S4f).** Core `operator/`: counts
  and sizes stay `Count` (`Int` | `Tuple`) through the operators. Python has no special case for a tuple count; its
  arithmetic just runs on it, so the port copies case by case: `operator::int` is the TypeError wherever Python
  computes with a count (`max(c, 0)`, `c - x`, `min(c, n)`...), `next_state_tuple` the cases where a tuple gets
  through: `reduce` of one child passes it on, `min` / `max` of two tuples (lexicographic, `[i64]` order) and `+`
  (concatenation; a tuple with an int is TypeError), `tuple * n` repeats (`times` / `repeat`, also the size of
  `MultiplyCounts`), `keep_counts('==')` is 0 and `'!='` the tuple, filters pass the source count on,
  `versus` multiplies a tuple left count by 0 / 1, `sort_pair_*_while` ignores a tuple right count once the
  "while" ended. A pairing operator that stores a tuple size in its state (`countdown = right_size`; versus in
  reverse order, max_pair early) gets `Statelet::TupleSize`, whose next state is the TypeError Python's first
  comparison raises; `UnsupportedOrder` / `OrderError` for a `None` size still come first. `CountFunction` (map_counts)
  takes and returns `Count`s (catalogue: `expression::count_value` / `value_count`; facade: `count_py`, a result
  that is an int or a flat tuple of ints, else a facade gap: a nested tuple, a bool). Built-in evaluators over
  tuple counts: S5d (above).
- Program layer: a multiset tuple expression is an operand of an expression's operators (reflected: `md + pool` is
  `pool.__radd__`), has no operators of its own (`md + md`, `md * 2`, `-md`, `+md` in `max_pair('==')` are
  TypeError) and no methods (`the_expression` now raises AttributeError for it); `pool_slice` on one is its
  `MultisetTupleSlice`; `keep_outcomes(md)` iterates it into a frozenset of `MultisetTupleIndex`es, which no outcome
  equals (an empty set). `MExpr::Op` is an `OpNode { operator, children, list_index }`: `list_index` marks a
  `MultisetKeep` given a list, whose `hash_key` is unhashable, so `add_inner` (any mixture) raises TypeError for an
  expression holding one (this was wrong before S4f for mixtures of leaves). `same_expression` compares operators by
  `_expression_key` (`same_operator`; a function by identity, `Arc` address; a `keep_outcomes` collection becomes a
  fresh closure per operator, so never equal) and the same node by identity (Python's `is` in the dict).
- `@multiset_function` keyword-only parameters: `mf_def` takes an optional `kwonly` list of names (old programs
  without it still run), `mf_kwarg(name)` reads one in the innermost traced body (VARS holds (args, kwargs)),
  `mf_call_kw(f, args, kwargs)` = `f(*args, **dict(kwargs))` (`star` among them is taken out by `_prepare` and never
  reaches the body: a kw-only parameter named `star` is always missing). Binding errors are Python's TypeErrors
  (unexpected / missing keyword). A nested call records its kwargs in the `RawResult`.
- Nodes (harness/grammar/multiset_function.py): `mf_kwarg`, `mf_call_kw` (bodies use the kwargs as `keep_outcomes`
  targets and count constants, sometimes as the result), `tuple_op_evaluate` (= `evaluate` over
  `tuple_count_expression`s: operators whose operands are whole MultiDeals or slices, then more operators on top;
  catalogue evaluators record / hand_sums / count_eq / seen / final_die). Function bodies now give tuple variables
  whole to operators for the any-count evaluators. multiset.py `gen_expression` makes mixtures of operator
  sub-expressions (6%, an inner repeated 60% of the time), so every expression node fuzzes operator equality.
  Catalogue: `count_max` (`max(counts)`). Fuzz by calling the generators directly (as for mf_call): at S4f, through
  icepool-run 16k tuple_op_evaluate, 30k mf_call_kw, 9k mixtures of operator expressions, 11k mx_evaluate, 8k
  mf_call; through the facade 6k tuple_op_evaluate, 5k mf_call_kw, 2k mixtures, 3k mx_evaluate, 1.5k mf_call:
  clean. (Facade fuzz after editing Rust: run-facade.sh rebuilds the extension on start, so the first program of
  each process can hit the 20 s timeout; run it once by hand first.) fixtures/s4f/edge-cases (142), facade cases in
  crates/icepool-py/tests (S4f block).
- Left: tupleize of a joint result holding a list (e.g. a kwarg that is a list returned as a constant) is
  `Unsupported` ("tuple outcomes holding lists or dice"); the generators avoid it.

- **Weightless generators (S4e).** Core `weightless.rs` = generator/weightless.py: `WeightlessSource<O>` wraps any
  `MultisetSource` (`Arc`), new `MultisetSource::Weightless`; `pop` returns the wrapped possibilities with weight 1
  and `Error::UnsupportedOrder` if two have the same `Count` (the driver's usual retry: other order / forward
  algorithm, then ConflictingOrderError). Python's pop is a generator that raises at the duplicate; the core pops
  the wrapped source fully first, so an error *from the wrapped pop itself* after a duplicate would win in the port
  (no built-in source raises mid-pop). There is no core generator type: the program layer has
  `MExpr::Weightless(KeepGen)` (only multiset generators have `weightless()`; on a `WeightlessGenerator` it returns
  itself, on anything else AttributeError; it is *not* a keep generator, so `+`, keeps, `*` build operators, and
  `_static_keepable` is False), `same_expression` compares the wrapped generators (`hash_key`
  `(WeightlessGenerator, wrapped.hash_key)`). Nodes (harness/grammar/weightless.py): `weightless` (kind mexpr) and
  the root `weightless_evaluate` (= mx_evaluate; leaves are small pools / deals, compound keeps, now and then a keep,
  so ConflictingOrderError comes up too). multiset.py `_leaf` wraps 5% of leaves in `weightless`, so every evaluation
  node (evaluate, mf_call, mx_evaluate...) meets weightless sources. Fuzz self-contained with `--ops
  weightless_evaluate,weightless,d,die,deck` (only ~10% of those programs are weightless: generate the node directly
  for volume, as for mf_call). At S4e: 30k direct weightless_evaluate programs clean through icepool-run, 16k
  through the facade, plus 11k mf_call / mx_evaluate / evaluate; fixtures/s4e (56).
- Facade: `WeightlessSource.__init__` builds `icepool_rs.weightless_source(_rust_source(wrapped))` (the wrapped
  source's `_rs`; since S5c a user source without one is a host source inside it); `_new_rust` + lazy `_wrapped` (`_wrap_rust_source`, which
  knows `WeightlessSource` now), `pop` / `size` / `outcomes` / `order_preference` / `is_resolvable` / `hash_key` on
  the core. `evaluator_rust._core_source` accepts `WeightlessSource`; icepool-py `core_source` / `source_object`
  know `RsWeightlessSource` (crates/icepool-py/src/weightless.rs). In the upstream suite every evaluation now runs
  on the core driver. JS: `MultisetExpression.weightless()` (node `weightless`), node-args.json regenerated.

- **map_to_pool, explode_to_pool, reroll_to_pool (S3c).** Core `to_pool.rs`: `explode_counts` (explode_to_pool's
  `rolls @ Die(single_data)`, Vectors `(i, 1)` / `(depth + 1, 0)`) and `reroll_composition` (reroll_to_pool's
  `map(step, initial_state, star=True, repeat=depth - 1)` on the core `TransitionCache` + `map_repeat`, `final_step`,
  `multiply_to_denominator`), both generic over the caller's `n @ die` (`matmul`, as count mode's Again does). Counts
  are `Value`s with Python's arithmetic (`rolls @ coin` keeps bools for one roll; float `rolls + max_rerolls` makes
  float quantities: TypeError like `Die()`; a non-int `repeat` is `range()`'s TypeError after the self-loop die;
  explode `depth < -1` always ends in `Die`'s TypeError on a float weight). Program layer `program/to_pool.rs`:
  `map_to_pool` (multiset expression args are *not* expanded, unlike `map`; zero args with star -> IndexError),
  `explode_to_pool` (`[explode] * x + [not_explode] * nx` = `Pool::new` with non-positive counts left out: a die
  put in 0 times would still add its outcomes), `reroll_to_pool` (`make_pool` over `die_pool`, `binary_op("+")`,
  `pool_highest_lowest`; a non-str mode falls to the ValueError with its `str`). Nodes (harness/grammar/to_pool.py,
  kind "mexpr", no gen: a mixture's repr has addresses): `map_to_pool`, `die_map_to_pool` (`f=None` = identity),
  `explode_to_pool`, `reroll_to_pool`, and the root `to_pool_evaluate` (= mx_evaluate; its generator builds one
  of them over small dice, 30% under a keep-like op). Catalogue functions only for map_to_pool (`Entry.pool`, never
  picked by `fn_node` at random, called by name): `n_of`, `pool_d`, `pair_list`, `reroll_or_pair` (JS ports in
  crates/icepool-wasm/js/test/catalogue.mjs). Fuzz: `--ops to_pool_evaluate` (self-contained); at S3c 5x5000 fresh
  clean through icepool-run and the facade; fixtures/s3c (113).
- JS suite at S3c (`crates/icepool-wasm/js/build.sh --dev`, then `node --test "test/*.test.mjs"`): fixtures/s3c pass
  through the JS API; 5 older fixtures fail there, as they did before S3c (l202610142332/first_vector.jsonl:1-2 "object
  is not subscriptable", m202610142332/mutant-killers.jsonl:13-15 "cannot pass [object Object] to the core"): for
  S6c. node-args.json was regenerated (it lacked the K6c nodes too).
- Fixed in passing (a fresh-seed gate fuzz found it): `Die::hash_key_compare` skipped only dict-equal items, but
  Python's tuple comparison skips `==` ones: `Symbols('b- b')` (zero counts) `== ()`. fixtures/s3c/hash-key-compare-symbols.
- Gaps found, not S3c's: `die_pool(die, 'x')` (a str keep index) and `Pool([pool, pool])` (a pool of pools: Python
  expands it) are Unimplemented in the program layer; `die.reroll(depth=1.5)` too. The generator avoids them.
- Facade: `icepool_rs.map_to_pool(transition, star, [arg_items...], kwargs)` = the loop, the host-callback calls and
  `pool in REROLL_TYPES` (Python's `in`); function.py then does `Pool(pool)` / `data[pool] += q` /
  `MultisetMixture(data)` as upstream. `icepool_rs.explode_counts(dens..., rolls, depth)` (depth extracted as i64:
  a float is range()'s TypeError) and `reroll_composition(dens..., rolls, max_rerolls, depth)` (after `depth ==
  'inf'`); die.py then calls `count_die.map_to_pool(lambda...)` / `pool_composition.map_to_pool(make_pool)` as
  upstream, so `make_pool` and the lambda are host callbacks. `Wallenius.deal` goes through the same map_to_pool.
  Oracle-process history: `map_to_pool(lambda: [E.A]).sum()` gave `1` in the long-lived oracle of the facade tests
  (the shared sum evaluator's room cache is keyed by `==`), so test cases use fresh outcomes.

- **S6: the JS facade (`crates/icepool-wasm/js`, npm package `icepool-wasm`).** Every JS method is one grammar
  node run by the program layer (`program::call(op, args)`, the node dispatch `eval` uses, now public), so the JS
  API computes what the oracle-checked nodes compute and needs no Rust per method. Layers: `src/facade.rs` (wasm
  exports `call`, `evaluate`, `release`, `live_handles`, `wire_result`, `from_wire`; a handle table of `PValue`s; the
  "JS-JSON" encoding documented at its top: ints as JSON or `{"$int"}` beyond 2**53, floats `{"float": repr}`, tuples
  = arrays, lists `{"$list"}`, vectors with their truth value, Symbols as dict items in order) and `js/core.js`
  (classes `Die`, `Pool`, `Deck`, `Deal`, `MultiDeal`, `MultisetExpression`, `AgainExpression`, value classes
  `Float` (integral floats, so they stay floats), `Fraction`, `Vector`, `Symbols`, `List`; FinalizationRegistry
  releases handles). Host hooks in the core: `program/host.rs` (`HostFunction` -> `Func::Host` in catalogue.rs;
  `HostEvaluator` + `evaluate`, run like the catalogue evaluators, `catalogue_evaluators::run` is generic now). JS
  callbacks go through the global `__icepool_wasm_host` (wasm-bindgen `js_namespace` imports, no snippets): a JS
  exception is kept on the JS side and the core gets `Error::Host`, so the very same exception is rethrown; an
  `UnsupportedOrder` thrown by a JS evaluator becomes the core's. A function's `f.length` is its required
  positional count (star inference); kwargs reach callbacks as a trailing options object (give it a default, or
  `length` counts it). JS evaluator states are values; plain objects become `JsonKey` host values (canonical JSON).
- **JS tests** (not in the gate: gate.sh is protected; task S6c asks for it): `crates/icepool-wasm/js/build.sh
  --test` = wasm-pack `--no-opt` builds into js/pkg-node + js/pkg-web, then `node --test "test/*.test.mjs"` and
  `tsc --noEmit` on test/types-check.ts. test/fixtures.mjs runs **every fixture through the JS API** (6238 at S6, 0
  failing, ~20 s): literals via `fromWire` (the core's decoder), nodes via the idiomatic method when the node's
  argument names match (test/node-args.json, regenerate when adding nodes: the command is in fixtures.mjs) else
  `call(op, args)`; catalogue functions and evaluators are the JS ports in test/catalogue.mjs (so fixtures exercise
  JS callbacks/evaluators against the oracle; `mapping` stays the core's); results re-encoded by JS and compared as
  compare.py does. `node test/fixtures.test.mjs fixtures/s4` runs a subset. When you add a node with a JS method,
  add its route in fixtures.mjs `IDIOMATIC` and its declaration in icepool.d.ts (api.test.mjs checks every export
  is declared). Hand-checked boundary cases: fixtures/s6/js-boundary. S6b and S6c (below) finished it.

- **S6b: `multisetFunction(f)` in the JS facade.** The program layer's `FunctionDef` body is `Body::Program` (mf_def)
  or `Body::Host(Arc<dyn HostFunction>)` (`program::host::multiset_function(f, params, var, kwonly)`): tracing calls
  the host with the parameter expressions and the kwargs, exactly where `eval(body)` runs for a program body (VARS is
  only pushed for programs). Everything else (raw results, nesting, joint results, constants, binding TypeErrors) is
  the S4d/S4f code unchanged. `host::evaluate` now records a `RawResult` when an input holds a parameter (`Host` owns
  an `Arc<dyn HostEvaluator>`), so JS evaluators work in bodies; the JS side keeps an evaluator in `evaluators` until
  the core drops its `JsEvaluator` (`Drop` -> `HOST.release`, one id space with functions). WASM export
  `multiset_function({"f": {"$fn"}, "params", "var", "kwonly"})`; tuple expressions reach JS as kind `tuple_expr`
  (`MultisetTupleExpression`, `hand`/`slice`; `MultiDeal` extends it). `MultisetFunction.call(...xs, { star, ...kw })`
  always uses `mf_call_kw`: `star` left out (undefined) is inferred, `star: null` is Python's `star=None` (no star).
  JS signature: `params` defaults to `f.length` (give the options parameter a default), `rest`, `kwonly`.
  Native unit tests of the host-body path: program/host.rs tests (a host body vs the same mf_def program).
- JS fixture runner: `mf_def` is `ip.multisetFunction` of a JS function evaluating the body program (JS `MF_VARS`
  stack; `mf_var` / `mf_kwarg` read it, Python indexing), `mf_call` / `mf_call_kw` are `f.call`, `mx_tuple_index` /
  `mx_tuple_slice` route on `MultisetTupleExpression`. A malformed mf_def falls back to the program-JSON call.
  At S6b: fixtures/s4d, s4f, s4e and s6b all pass through JS; 27k generated mf_call / mf_call_kw / mx_tuple_evaluate /
  tuple_op_evaluate programs (`G.NODES[op].gen` + `tool.oracle` into a jsonl, then fixtures.mjs `check()` per line)
  clean through JS and natively. Whole JS suite: 2 failures left, both l202610142332/first_vector.jsonl (S6c).
- Fixed: `mf_var(-1)` raised IndexError; Python's `_VARS[-1][0][i]` counts from the end (fixtures/s6b).
  `Pool.keep(array)` in JS passed a list, so a mixture of such keeps raised "unhashable" (s4f/edge-cases:104):
  arrays are tuples there too now.

- **S6c: the rest of the Python surface in JS.** New nodes: `die_sequence` (population.py; Rust
  `collection::die_sequence` = cartesian_product of `range(rolls)` copies, `()` for 0 rolls), `die_stochastic_round`
  (function.py; `map` with the internal `Func::StochasticRound(m)`, no catalogue name), `wallenius_deal_expr`
  (function.py, kind "mexpr", no gen: `to_pool_evaluate`'s generator builds it 15% of the time; `wallenius_deal` is
  now `method_evaluation(wallenius_deal_expr(...))`). Wallenius mapping values may be Python lists (unpacked; a list
  weight is unhashable TypeError). `function::from_cdf(has_pdf, cdf, outcomes, denominator)` is from_rv's body for
  any rv: the catalogue `Rv` and `program::host::from_rv(cdf: Arc<dyn HostFunction>, has_pdf, ..., kwargs)` (a host
  rv's bound `cdf` is just a HostFunction; WASM export `from_rv`). `program::host::mapping(items)` = Python's
  `dict(items)` as `Func::Mapping`; JS-JSON `{"$map": [[k, v], ...]}` (JS -> Rust only) is how core.js `enc`
  sends any JS `Map` (keys compared by Python `==`: 1 / 1.0 / true one key, equal arrays one key). JS constructors
  that take a Map themselves (`new Die(Map)`, Pool, Deck, dPool, Symbols.from, Wallenius) still send its items.
- JS API added: `fromRv(rv, outcomes, denominator, kwargs)` (`"pdf" in rv` is hasattr; `cdf(xs, kw)` called once),
  `Wallenius(data | Map).deal(n)` (constructor validates with a hand of 0, as Python raises in `__init__`), Die
  `keep(rolls, index)`, `keepSlice(rolls, start, stop, step)`, `sequence`, `meanTimeToSum`, `stochasticRound`,
  `mapToPool`, `explodeToPool`, `rerollToPool` (`maxRerolls`/`depth` Infinity = 'inf'), free `mapToPool`.
  fixtures.mjs routes all of these idiomatically (also `from_rv` through catalogue.mjs `RVS`, JS ports of rvs.py,
  and the catalogue `mapping` as a JS `Map`). catalogue.mjs `first` indexes Vectors (fixed the 2 first_vector
  failures). At S6c `build.sh --test` is fully GREEN (all fixtures, 18 API tests, tsc); 8000 generated programs of
  the routed nodes also pass through JS. The suite is still not in a gate: asked in QUESTIONS.md (S6c).
- Not ported (found here): `limit_denominator` with a float bound (`stochastic_round(x, max_denominator=2.0)`, also
  `coin`): Unimplemented; Python goes through float arithmetic in `Fraction.limit_denominator`.

- **S5: the check that nothing of Vector / Symbols / wallenius / function.py / `__init__` exports is left.** S5a/S5b
  covered the area; the open Python work is all in tasks (FACADE-AUDIT.md: S3c, S4e, S4f, K6c, S5c). S5 compared
  `__all__` with the grammar and added nodes for the exports that had none: `tupleize` (collection.py; Rust
  `collection::cartesian_product`, shared with `vectorize`), `middle` and `die_middle` (pool.py; compose `pool` /
  `pool_middle` / the sum evaluation), `map_function` (maps.py; given `f` directly upstream binds only `**kwargs`,
  so `star` is dropped, the decorator form keeps it) and `die_method` (die_methods.py: `die[x]`, `x in die`, `len`,
  iteration, the `CountsKeysView` / `ValuesView` / `ItemsView` indexing, slicing and `==`, min/max_outcome,
  is_empty). `map_to_pool` has nodes since S3c. Fuzz `die_method` together with die nodes
  (`--ops die_method,d,die,...`): alone the generator cannot build its die argument and tests nothing.
- **icepool.d / icepool.z are `@functools.cache`d** (keyed by `==` for non-int arguments): in the long-lived fuzz
  oracle `z(True)` made `z(Fraction(1))` return a die. Nodes calling d / z run `basic.fresh_dz()` first
  (DIVERGENCES.md). Any new node calling them, or another `@cache`d upstream function, must clear the cache too.

- **S4: the check that nothing of Deck / Deal / MultiDeal / multiset operators / expressions is left.** S4a-S4d
  covered the area; what is still open is all in tasks: weightless generators (S4e), operators over tuple counts,
  mixture inner equality, @multiset_function kwargs (S4f), evaluators called back from the driver incl. argsort,
  leximin/leximax, sum(map=), wild largest_count (K6c), `Deck.map` and MultiDeal built-ins (S5c). S4 added the
  last uncovered public API as nodes (harness/grammar/deck.py): `deck_method` (`additive_union` / `difference` /
  `intersection` / `union(*args)`, which always raise TypeError on 3.13, see DIVERGENCES.md; `symmetric_difference`
  = `^`; `deck[outcome]` via `Die::position`, KeyError / unhashable TypeError; `keys` / `values` / `iter`) and
  `deal_str` (`(repr, str)` of a Deal or MultiDeal; `str` is a header plus `crate::format::format(deck, "Deck",
  "")`). `wire::deck_repr` and `multi_deal::hand_groups` are `pub(super)` for it. Fuzzing these needs the deck
  nodes allowed too: `fuzz --ops deck_method,deal_str,deck,d,die,deck_op,deck_mul,add` (with only the new ops,
  `g.make("deck")` has nothing to build and 0 programs run). Fixtures: fixtures/s4/deck-methods-deal-str.

- **S3: the check that nothing of Again count mode / map(repeat) / Markov chains is left.** The facade side was
  complete (FACADE-AUDIT.md: again.py, map_tools/* on the core) except a bare `AgainExpression()` in count mode,
  which raised NotImplementedError: the core's bare `Again` now carries `is_additive` (`Again::Again {
  is_additive }`; `icepool.Again` is `true`), so count mode raises Python's ValueError at it, in outcome order
  (cases in crates/icepool-py/tests). Program layer: `map.rs` `arg_items` takes a deck's items too
  (`_items_for_cartesian_product` is on every Population; a deck was passed as-is before), and `expand_arg` is
  `arg.expand() if isinstance(arg, MultisetExpression)` (pool, deal, non-tuple expression -> `evaluate::expand`),
  applied to every arg of `map` / `map(repeat)` and to the extra args of `map_and_time`, as function.py does.
  `mean_time_to_absorb` does *not* expand: a multiset expression reaches the function as-is there (still
  `Unimplemented` in the program layer). markov.py generators now put decks (and pools, where expanded) among the
  args; fixtures/s3/map-populations. JS callbacks for map belong to S6 (the JS facade).

- **S2: the check that nothing of population/base.py and die.py is left.** Every public method of `Population`,
  `Die`, `DieWithTruth` and format.py now has a grammar node, and runs on the core under `ICEPOOL_BACKEND=rust`
  except what FACADE-AUDIT.md assigns to S3c / S5c (`map_to_pool` and friends, `append` / `remove` /
  `to_one_hot`, `mean_time_to_sum`). S2 itself: `Die.sign` (was a Python callback per outcome) and `Die.cmp` are
  core methods (`Die<Value>::sign`, `cmp_die`: not `cmp`, clippy confuses it with `Ord::cmp`); nodes in
  harness/grammar/die_extra.py: `zero_outcome`, `standardized_moment` (integral k only: fractional k on a negative
  deviation is complex, task V1), `sign`, `die_cmp`, `is_in`, `die_count`, `if_else` (all three again modes),
  `die_repr` (`DieWithTruth(...)` for `==` results). `is_in` / `count` / `if_else` stay compositions of `map` in
  the facade (the container and `bool` are Python's), as upstream writes them. `map.rs` `contains` is the
  program layer's `x in collection`; reuse it.
- **Complex numbers (V1).** `value/complex.rs` is CPython 3.13's complexobject.c bit for bit: `+ - * /` are the
  plain formulas (`_Py_c_quot` is Smith's algorithm; 3.13 has *no* infinity recovery and no mixed real/complex
  arithmetic: `1j * inf` is `(nan+infj)`, both came in 3.14), a real operand is `complex(float(x), 0.0)` first
  (OverflowError for a huge int/Fraction), `**` is `c_powi` (repeated squaring) for integral exponents with
  |n| <= 100, else `_Py_c_pow`. The oracle's binary computes `len *= exp(-at*b.imag)`, not the `len /= exp(...)`
  one remembers from the source: trust the disassembly (`objdump -d` the uv python, symbol `_Py_c_pow`), which is
  also how `math.hypot` was ruled out (it is not libm's hypot; `_Py_c_pow`/`abs` call libm's). errno: `cos`/`sin` of
  an infinite phase set EDOM, so `(-1+1j) ** 1e308` raises ZeroDivisionError ("0.0 to a negative or complex
  power"); an infinite part is OverflowError. `//`, `%`, bitwise, `<` and `~`/`trunc`/`floor`/`ceil`/`round` raise
  TypeError without converting. Nodes (harness/grammar/complex_num.py): `complex(re, im)`, `cx_binary`, `cx_unary`
  (operator.<name> on values, Rust side = collection.rs `binary`/`unary`); operators.py `scalar` is sometimes a
  complex. JS: a `Complex` class (`{"complex": [re repr, im repr]}` both ways, `String(z)` is Python's repr).
  The libm calls are `f64::hypot/powf/atan2/exp/ln/cos/sin` (see "Float math" below).
- **function.py helpers, reduce/accumulate, from_rv, Wallenius (S5b).** Core `function.rs`: `z`, `pointwise(dice,
  max)` (= pointwise_max/min *after* `harmonize_denominators`), `weighted_lcm(denominators, weights: &[Value])`
  (math.py's, with Python's `w > 0`, `math.gcd` TypeError for non-ints, `lcm * w // d` as Value ops, `zip`
  truncation), `coin_data(n, d)` (the dict before `Die(data)`), `py_min_max` (builtin `min`/`max`: `item < best`,
  ValueError when empty), `consecutive(first, second)` (the two chains Python iterates; ranges over 2**20
  Unsupported), `one_hot`. `value/builtins.rs`: `fraction(x)` (= `Fraction(x)`; a str is Unsupported) and
  `limit_denominator` (CPython 3.13's algorithm, unit-tested on its doctests).
- Program layer `program/function.rs`, nodes in harness/grammar/function.py: `z`, `dz_attr` (`getattr(icepool,
  name)` through function.py's module `__getattr__`: `int()` parsing of `d<...>` with sign / underscores / spaces,
  a ValueError from `d(0)` swallowed into AttributeError, `''` IndexError; non-ASCII digits Unsupported; the
  generator never makes a name icepool defines, e.g. `d`), `coin`, `stochastic_round`, `one_hot`, `pointwise`
  (`args, max, spread`), `extreme_outcome` (min/max_outcome), `consecutive`, `sorted_union`,
  `harmonize_denominators` (kind "value": a tuple of dice), `reduce` / `accumulate` (map per step; reduce of
  nothing raises StopIteration, here `Error::Value("StopIteration")`: error types are informational), `from_rv`
  (random variables from harness/catalogue/rvs.py: `discrete_uniform` / `continuous_uniform`, duck-typed like
  scipy's; Rust mirror `Rv` in function.rs), `wallenius_deal` (`getattr(Wallenius(data).deal(n), method)()`).
  Wallenius in the program layer: `Deck().append` via `deck_from_data`, `_wallenius_weights` with a per-call cache
  (Python's `@cache` gives equal results), `map_to_pool` done inline (deals summed with `binary_op("+")`, inners
  merged by `add_inner`; the `map_to_pool` nodes came with S3c). Hands deeper than 40 cards are Unsupported
  (Python's recursion limit). Fuzz: `--ops z,dz_attr,coin,stochastic_round,one_hot,pointwise,extreme_outcome,
  consecutive,sorted_union,harmonize_denominators,reduce,accumulate,from_rv,wallenius_deal,d,fn,die_typed`; at
  S5b 5000 fresh clean both ways, 218 edge-case fixtures in fixtures/s5b/function.
- Facade python/icepool/function.py: `coin` (`Fraction(n)` / `limit_denominator` stay Python; `coin_data` then
  `Die(dict)`), `one_hot`, `pointwise_max/min` (after the Python `harmonize_denominators`), `min/max_outcome`
  (`_iter_outcomes` stays Python), `consecutive` (both chains materialised, so `consecutive(iterator)` still
  raises as upstream: DIVERGENCES.md), `sorted_union` (`[list(arg) ...]`; order by first-seen, not set order, so
  partially ordered outcomes may sort differently: the K5 gap) and `harmonize_denominators`' scale factors
  (`weighted_lcm`) call icepool_rs (crates/icepool-py/src/function.rs). Unchanged because they only compose core
  operations: `d`/`z`/`__getattr__` (`Die(range)`), `stochastic_round` (`math.floor` + `coin` + `int + Die`),
  `from_cumulative` (subtraction loop over user ints, then `Die(dict)`), `from_rv` (`rv.cdf` is the user's),
  `reduce` / `accumulate` (loops over the core `map`), `Wallenius` (Deck / Die / Deal / mixtures on the core;
  its `map_to_pool` loop is the core's since S3c). Facade audit: FACADE-AUDIT.md.

- **Vector and Symbols as native values (S5a).** Core `value/vector.rs` (`Vector { data: Option<Arc<[Value]>>,
  truth: Option<Arc<Value>> }`; `data: None` is `VectorWithTruthOnly`, whose every use but `bool()`/`repr()` raises
  ValueError) and `value/symbols.rs` (`Symbols { data: Arc<[(char, Value)]> }` in the *dict's insertion order*:
  the constructor sorts, operators build with `_new_raw` and do not; zero counts stay keys, so `len`/`bool` count
  them while `==`/`str`/hash do not; counts are `Value`s with Python's arithmetic). `ops::collection_binary` is
  Python's dispatch when either operand is one (left method, then the right one's reflected method or swapped
  comparison, then TypeError; `str % x` formats first). `Value::eq_op` is the operator `==` (a Vector compares
  elementwise, `Symbols('ab') == 'ab'` is True, a failed `Symbols(other)` falls back to identity = unequal);
  `PartialEq` stays *dict* equality (a Vector/Symbols only equals its own kind, as their hashes differ from
  everything else's). `Outcome::py_eq` is `bool(a == b)`: `Counts::new` sorts `(outcome, quantity)` pairs with it,
  as Python's `sorted(items)` does, so `Die([Symbols('a'), 'a'])` keeps both, ordered by quantity.
  `Value::truthy()` now returns `Result` (a Vector without a truth value raises; so do host objects), and
  `HostObject::truthy` too. A vector's truth value is the raw object of `op(self._data, other._data)`:
  `Value::binary` on two tuples returns the first differing elements' comparison *object* (Python's tuple
  comparison), so nested vectors don't sort (DIVERGENCES.md). `Die::hash_key_compare` returns a `Value` likewise.
  `Value::check_hashable` (VWTO anywhere inside) is part of `check_key`. `wire::enc` is fallible (encoding a VWTO
  raises, as encode.py's iteration does).
- Program layer `program/collection.rs`, grammar harness/grammar/collection.py: `die_vectors`, `die_symbols`
  (plain `Die(outcomes)` of literals; "die" kind, so every die node gets them), `vectorize` (no Reroll in the
  generator: Reroll would flow into other die nodes), and on kind "collection": `vector`, `symbols`, `coll_binary`
  (`operator.<name>`, incl. eq/ne), `coll_unary` (incl. round), `coll_truth`, `vector_method`, `symbols_method`
  (`keys`/`values`/`len`/`repr` expose dict order and zero counts, which the wire format's `str()` hides).
  `operators.operand` returns a vector/symbols literal 2% of the time. `py_iter` iterates vectors (elements) and
  Symbols (keys). `Symbols // die` is `count_subset` over the die's items (a die is an iterable mapping).
  `PValue::DieWithTruthError(err, truth)`: Python's `DieWithTruth` computes its data lazily, so `bool(die_of_vectors
  == other_length_vector)` works while its data raises; `eval` raises the error for any node but `truth` /
  `coll_truth`, and so do list elements and the result encoder. Catalogue `is_even` returns `x % 2 == 0` as is.
- Facade: python/icepool/collection/vector.py routes `binary_operator`, `reverse_binary_operator`,
  `unary_operator` to `icepool_rs.vector_*` (after the Population/AgainExpression `NotImplemented` check; the op
  is classified like `Die`'s: standard operators native, anything else a host callback per element, the truth value
  `op(tuple, tuple)` through the same callback). symbols.py: constructor (`symbols_from_str/_iter/_mapping` return
  the sorted items; the facade builds the `defaultdict`), every operator and method (`symbols_op`, `symbols_unary`,
  `symbols_count_subset`, `symbols_getitem`) after Python's own juggling (`Symbols(other)`, `isinstance(other,
  int)`, `Counter(divisor)`). Container protocol, `__hash__` (Python's own formula), `__bool__`, `append`,
  `concatenate`, `__getattr__` stay Python. `expand.cartesian_product(outcome_type=Vector)` uses
  `icepool_rs.vectorize`. convert.rs: *exact* `Vector`, `VectorWithTruthOnly`, `Symbols` cross as native values
  (truth value included; back via `Vector(items, truth_value=...)`, `VectorWithTruthOnly(t)`, `Symbols._new_raw(
  defaultdict(int, items))`); a `Symbols` with a non-character key (only `s[tuple]` makes one) stays a host object
  and its methods raise NotImplementedError, as does `s[key]` for a non-str key of length != 1.
- Gaps: tuple equality inside the core is `PartialEq`, so `(Symbols('a'),) == ('a',)` is False where Python says
  True (only reachable with Symbols next to equal strs in tuples). Python's identity shortcut in `==` is not
  modelled (as for NaN). The pinned oracle's `Symbols.__getitem__` already uses `.get` (ANALYSIS.md's "inserts
  missing keys" bug is fixed there), so there is no defaultdict-insertion quirk left to copy.

- **@multiset_function, tuple expressions, mixtures on the core (S4d).** Core `expression.rs` drives a `CallTree`
  (`flats` + `calls`, = Python's Dungeonlet/QuestletCallTree) with two new dungeonlets: `Parameter{arg_index,
  star_index}` (reads the *enclosing call's* output counts/sizes, `arg[star]` with Python subscript errors) and
  `TupleItem{child, index: builtins::Index}` (= MultisetTupleIndex/Slice; a `None` child size is the TypeError of
  unpacking Python's bare `None`). What a call tree produces is a `CountTree` (`Leaf(counts)` at a call without
  calls, else one subtree per call); statelets of the whole tree stay one flat vector (pre-order). The driver
  (`evaluate_call_tree`; `evaluate_expressions` wraps it) calls the new trait methods `initial_state_tree` /
  `next_state_tree` / `final_outcome_tree`, whose defaults take `CountTree::Leaf` (a plain evaluator never sees a
  tree). `evaluator/multiset_function.rs`: `MultisetFunction { call: Call, tupleize }` walks `Call::{Evaluator(Box<dyn
  DynEvaluator>), Function(inner), Joint(inners)}` with the count tree: Function passes its inner's state through,
  Joint's state is `CallState::Joint` (Python's tuple; any inner Reroll -> Restart, later inners not stepped),
  finals of a joint are `tupleize`d by the caller's closure. Leaves are type-erased (`Erased { evaluator, map_final }`,
  states as `AnyState`: the type's own Eq/Hash behind `dyn`); a constant result is just a leaf evaluator with no
  inputs (`ConstantDungeon`). Tracing (calling the body with parameters) is never in the core.
- Program layer `program/multiset_function.rs`: nodes `mf_def` (lazy `body`, special-cased in `program::eval` like
  khepri; `params` positional + `var` = `*rest`), `mf_var` (thread-local stack of the innermost traced body's
  arguments), `mf_call` (`f(*args)` / `star=`). Any evaluation whose inputs hold a parameter returns
  `PValue::RawResult` (evaluate.rs `run` / catalogue `run` check `in_function` first; that is why `run` now takes
  the evaluator by value: it is erased into an `Arc` leaf). `prepare_function` = `MultisetFunctionEvaluator._prepare`:
  star inference (`params > len(inputs)`), `_make_param` (a tuple expression makes `MExpr::TupleParam`), the arity
  TypeError, then single (`convert_die_to_raw_result`: an expression result is TypeError) or joint (anything else
  iterated; `[]` is the ValueError of unpacking an empty `zip`). Nested functions re-trace per outer preparation, as
  Python does. Python quirks copied: `MultisetTupleIndex.hash_key` leaves out the child (DIVERGENCES.md), `x[i]` with
  a non-int i on a tuple expression returns `None`, evaluation methods on tuple expressions are AttributeError.
- Mixtures: `MExpr::prepare` now returns every preparation (`Part` = dungeonlets, sources, weight;
  `itertools.product` over children, and **child flats are concatenated with their absolute child positions
  shifted** (`shifted`): forgetting that silently broke operators over operator children). `evaluate::mix` is the
  `final_data[die] += weight; Die(final_data)` end of `evaluate`. `MExpr::Mixture` (node `mx_mixture`, list form
  only): inners merged by `same_expression` (= hash_key equality; every operator since S4f). A `_static_keepable` mixture (all inners keep generators or such mixtures) maps
  `keep` / `[]` / `lowest` / `highest` / `multiply_counts` / `*` / `-` over its inners (`map_mixture`); `keep(int)`
  then raises AttributeError (the inner results are dice); a MultisetKeep result with a *list* index is unhashable
  (TypeError) when put in the new mixture's dict.
- Facade: `evaluator_rust.rust_call_tree` describes MultisetFunction(Joint)Dungeon/Quest trees as nested
  `('function'|'joint'|'builtin'|'host', flats, ...)` (flats now also hold `('param', arg, star)` and
  `(('tuple_item', index), child_indexes)` nodes); `icepool_rs.evaluate_tree` builds `CallTree` + `MultisetFunction`
  (built-in leaves through `with_builtin` + `BuiltinVisitor`, shared with `evaluate_builtin`; other leaves and
  ConstantQuest are `HostEvaluator`s with the function's `inner_kwargs`; with any MultiDeal source every leaf is
  host, as before; joint finals go through `icepool.tupleize`). A builtin leaf is recognised from the quest's
  `initial_state_main.__self__` (the dungeon may come from the evaluator's `_cache`). Mixtures need nothing in the
  facade: Python's `_prepare` yields ordinary evaluations. In the upstream suite every evaluation but two
  (`weightless()`, on the core since S4e) now runs on the core (1797 plain, 49 call trees).
- `CompoundKeepSource.outcomes()` is now lazy and fallible (`MultisetSource::outcomes() -> Result<&[O]>`): Python
  sorts the inner outcomes only when asked, so an evaluation with an unresolvable source returns `Die([])` before
  a mixed int/str union raises (was a TypeError in the port, found by S4d's fuzz, fixture in fixtures/s4d).
- Fuzzing: the official fuzzer rarely reaches `mf_call` (leaves dominate the kinds). Generate the node directly
  (harness `G.NODES['mf_call'].gen(g, depth)` in a loop against `tool.oracle` / `tool.Port`, `ICEPOOL_RUN` for the
  facade): at S4d 8k mf_call, 4k mx_tuple_evaluate and 8k mixture-heavy mx_evaluate programs clean both ways;
  fixtures in fixtures/s4d (92). The generators keep each function's arity and `star` consistent with the body's
  view of its variables (a tuple variable is only indexed, never an operand): see `_call` in
  harness/grammar/multiset_function.py. K6c's evaluators (argsort, comparisons, sum(map), wild largest_count, lexi)
  were not done here: they already run on the core through host callbacks in the facade; K6c gives them native
  evaluators and nodes.

- **Multiset operators and CompoundKeepGenerator on the core (S4c).** Core `operator/` = operator/*.py, one file
  per icepool file (binary, adjust_counts, filter_outcomes, keep, pair, versus, debug): `Operator<O>` is both the
  `BodyDungeonlet` (`next_state`) and the `BodyQuestlet` (`initial_state`) of an operator node; its local state is
  a `Statelet` (hashable enum: `None`, `Keep{keep_tuple, keep_more}`, `Versus`, `SortPair`, `SortPairWhile`,
  `MaxPairLate/Early`). Counts are `Int` (`Count::Int`; `i64` before K6d), or tuples since S4f (above); a non-int
  `map_counts` result or constant are `Unsupported`. `map_counts` functions and `keep_outcomes` callables are trait objects
  (`CountFunction`, `OutcomePredicate`, `Send + Sync` or `PValue` stops being `Send`); a collection target is
  `OutcomeFilter::Set` (`==` membership). `lexi.rs`: `compute_lexi_tuple*` (comparison / extra are strings, an
  invalid one raises ValueError where Python's `match` does: at evaluation for sort_pair, at construction for
  versus / max_pair). `expression.rs` = the evaluation side of multiset_expression_base.py: an input is a `Flat`
  (postfix `Dungeonlet::{FreeVariable, Operator{operator, children}}`, children as absolute positions), statelets of
  all operator nodes of all inputs in one `Statelets` (free variables keep none). `compound_keep.rs`:
  `CompoundKeep` holds the inner *sources* (immutable, so equal to remaking them) + keep tuple, and
  `CompoundKeepSource` (outcomes = sorted union computed once; new `MultisetSource::CompoundKeep`).
- The driver (`evaluator::evaluate_expressions(evaluator, flats, sources, check_depth)`; `evaluate_with_depth_check`
  is it with one free variable per source) carries `(statelets, main state)` exactly like Python's
  `defaultdict(lambda: defaultdict(int))`: results are `FinalStates` = statelets -> {state: weight}, statelet groups
  in first-seen order (finalize iterates them nested, which can change which spelling of an equal final outcome Die
  keeps), a statelet entry created even if all its main states rerolled. The dungeonlets step once per (pop,
  previous statelets), then the evaluator once per main state, as `DungeonletCallTree.next_state` does. Sizes from
  the questlets (`expression::initial_state`) are what `initial_state` / `final_outcome` get, e.g. `None` for
  `unique()`.
- Python quirks copied: an operator with no children is fine (`positions[-1]` sits in a lazy generator); its
  `reduce` raises TypeError only once an outcome is seen. `MultisetKeep` with a negative child size slices
  `keep_tuple[:size]` from the end and `[0] * size` is empty (port in `operator/keep.rs` with Python slice / list
  assignment semantics, IndexError past the end). `(keep_more,) * more` makes bool elements: harmless, they are only
  summed. `sort_pair` raises a plain `OrderError` (not Conflicting): core `Error::Order` with an `"OrderError: "`
  prefix (icepool-py maps it). `MultisetForceOrder` message prints the int.
- Program layer `program/expression.rs`: `MExpr { Gen(KeepGen), MultiDeal, Op(Arc<(Operator, children)>) }`,
  `PValue::Expr` (compound keeps and operators; pools / deals / multi-deals keep their own variants, see
  `into_pvalue`). `KeepGen` gained `Compound`; every evaluation takes `MExpr`s (`evaluate::run`, catalogue
  evaluators, `implicit_convert` now returns `MExpr`) and `prepare`s them. Python's dispatch is in `binary_op`
  (`KeepGenerator.__add__` = `self.additive_union`: `Pool.additive_union` merges pools or falls to
  `KeepGenerator.additive_union` = compound if all keep generators have no negative keeps, else
  `MultisetAdditiveUnion`; reflected operators for a list / tuple / str on the left, `__rxor__` swaps). The keep-like
  pool nodes (`pool_keep`, `pool_slice`, `pool_highest/lowest`, `pool_middle`, `pool_mul`, `pool_neg`, `pool_add`)
  take any expression: a keep generator without negative keeps stays one, anything else is `MultisetKeep` (an int
  index on those is the keep evaluator with that index).
- Nodes (harness/grammar/multiset.py, kind "mexpr", no gen: an expression's repr has an address): `mx_binary` (`a o
  b`; the argument is `o` because `op` is the node key), `mx_method` (`MultisetExpression.<name>(*args)`),
  `mx_counts` (`o` in mul/neg/pos/floordiv/mod/unique/keep_counts/multiply_counts/divide_counts/modulo_counts),
  `mx_filter`, `mx_map_counts` (generated with catalogue `count_diff` / `count_if_gt_k` only: add2 / max2 over bool or float outcomes make non-int counts, a gap), `mx_sort_pair`,
  `mx_max_pair`, `mx_versus`, `mx_force_order`, and the root `mx_evaluate` (`getattr(expr, method)()`).
  `gen_expression` builds trees of up to 3 operators over `gen_pool(..., expressions=False)` leaves; `gen_pool(...,
  deals=True)` returns an expression 20% of the time, so every evaluation node fuzzes expressions too. Fuzz with
  `--ops mx_binary,mx_method,mx_counts,mx_filter,mx_map_counts,mx_sort_pair,mx_max_pair,mx_versus,mx_force_order,mx_evaluate,d,deck`
  (deck is needed: deals are leaves). At S4c: 20k of those clean through icepool-run and the facade, 469 edge-case
  fixtures in fixtures/s4c. The catalogue evaluator `record` (evaluate node) shows every count and size an
  expression passes: the best probe for operator bugs.
- Facade: `evaluator_rust.rust_flats(dungeon, quest, sources)` (was `is_rust_evaluable`) describes each prepared
  input as `None` (free variable over a core source: pool / deal / compound keep / multi-deal source) or `(spec,
  child_indexes)`; `_operator_spec` names the exact operator class and the attributes it reads. **The spec comes from
  the questlet's operator**: a dungeon found in the evaluator's `_cache` is only equal by `_dungeonlet_key`, which
  leaves out what only `_initial_state` reads (a `MultisetKeep`'s index...), so its dungeonlets may belong to an
  earlier expression (the check is that both operators have the same type and `_dungeonlet_key`). icepool-py
  `expression.rs` parses specs into core `Operator`s (`map_counts` / predicate callables are host callbacks; a
  `keep_outcomes` collection is read out of the constructor's closure cell `target_set`), and `evaluate_builtin` /
  `evaluate_host` take `flats`. `CompoundKeepGenerator` holds `icepool_rs.CompoundKeep` as `_rs` (built from the
  inners' `_make_source()._rs`; `keep` (keep.py), `multiply_counts`, `_set_keep_tuple`, `hash_key`, `_make_source` on
  the core); `CompoundKeepSource._new_rust` pops / sizes / prefers order on the core, `inner_sources` and
  `keep_tuple` lazily via `__getattr__`. In the upstream suite 1796 evaluations ran on the core at S4c; since S4d all
  but `weightless()` (on the core since S4e).
- Facade gaps (NotImplementedError where Python answers): a `map_counts` function returning a non-int (a bool too:
  Python keeps bool counts, which can leak into results such as `all_counts`), operator constants that are not ints
  (`unique(1.5)`, `unique(True)`, `keep_counts('>', 0.5)`), `MultisetKeep` indices that are not a slice / list /
  tuple of ints and `...`, slice bounds that are not ints. icepool-run says unimplemented for the same.

- **MultiDeal and tuple sources on the core (S4b).** Core `multi_deal.rs` = generator/multi_deal.py: `MultiDeal<O>`
  (deck + `hand_groups: [(usize, usize)]`; `new` checks the total like Python, then refuses negative sizes as
  `Unsupported`: Python accepts them and later reads icepool's cached `comb_row` with negative indices, so the answer
  depends on the process's history) and `MultiDealSource<O>` (outcome window like `DealSource`; `pop` ports the
  `iter_hypergeom` split (`math::iter_hypergeom`), the per-group descending sort + `groupby` regrouping and the skip
  branch). New `MultisetSource::MultiDeal`; `MultisetSource::pop` now yields `pool::Count` (`Int(i64)` |
  `Tuple(Arc<[i64]>)`) and `size()` is `Option<Count>`. The driver calls the new trait methods
  `initial_state_counts` / `next_state_counts` / `final_outcome_counts`; their defaults convert to the old `int`
  methods (a tuple: `next_state_values` / `reads_sizes`, S5d). `HostEvaluator`
  (icepool-py) and `CatalogueEvaluator` override them: counts/sizes reach Python (or the Value bodies) as tuples.
- `CatalogueEvaluator` bodies are now written over counts as `Value`s (so a tuple count meets Python's operators:
  mostly TypeError, `count_eq` with an absent k is fine). New catalogue evaluators `hand_sums` (per-hand sums; a
  pool input raises TypeError in initial_state) and `record` (every `(outcome, *counts)` seen, then `(record,
  sizes)`; works for any inputs). They are also picked by the old `evaluate` node.
- Program layer `program/multi_deal.rs`, `PValue::MultiDeal` (repr `MultiDeal(Deck({...}), hand_groups=((2, 1),))`).
  Nodes (harness/grammar/deck.py): `multi_deal` (`deck.deal(hand_sizes)`; the old `deal` node handles a non-int
  too), `deal_groups` (`deck.deal_groups(*map(tuple, groups))`), both kind "multi_deal" without gen;
  `multi_deal_info`, `multi_deal_pop`, `multi_deal_evaluate` (= `evaluate` with a multi-deal among the args,
  generator `_gen_multi_deal_evaluate`). `gen_multi_deal` keeps at most 6 cards dealt. Bool / non-int hand sizes
  are unimplemented in icepool-run (Python shows `True` in hand_sizes and the repr). Fuzz with deck ops in `--ops`,
  e.g. `--ops multi_deal,deal_groups,multi_deal_info,multi_deal_pop,multi_deal_evaluate,deck,d,die_pool,pool_keep,deal`
  (and `multi_deal,deal_groups,multi_deal_evaluate,deck,deck_op,deck_mul` for mostly evaluations).
- Facade: `MultiDeal.__init__` keeps the hand groups and the total check in Python, then `_rs =
  icepool_rs.multi_deal_new(deck._rust_deck(), [tuple(g) for g in hand_groups])` (crates/icepool-py/src/
  multi_deal.rs; a non-int size -> NotImplementedError, a negative one too: facade gaps). `hand_sizes`,
  `total_cards_dealt`, `len`, `hash_key`, repr stay Python; `denominator` and `_make_source` are the core's.
  `MultiDealSource._new_rust` (deck / hand_groups lazily via `__getattr__`) pops, sizes (a tuple), outcomes (a
  tuple, not a keys view, as for DealSource), resolvability and `hash_key` on the core. `is_rust_evaluable`
  accepts a `MultisetTupleGeneratorQuestlet` over a `MultiDealSource`; since S5d a built-in evaluation over one runs
  natively too (`builtin_spec` with `tuple_counts`; argsort / lexi still through `evaluate_host`). A hand of a multi-deal (`deal[0]`, `MultisetTupleIndex`) is an expression node on the core since
  S4d (`Dungeonlet::TupleItem`).
- Upstream bug copied: `MultiDeal.denominator()` for 3+ hands is not the multinomial (DIVERGENCES.md).

- **Deck and Deal on the core (S4a).** Core `deck.rs` = population/deck.py: `Deck<O>` wraps its cards as a `Die`
  (`population()`: every `Population` method means the same on both, so statistics / pops / hashing are shared),
  `Deck::new(args, dups)` = `expand_args_for_deck` (`creation_args::merge_duplicates`: dups add up, zero dups and
  zero-quantity outcomes of a die argument contribute nothing, not even the key). The operators go through a
  port of Python's `Counter` (`deck::Counter`: insertion order, signed `IBig` counts, missing key = 0, first key
  spelling kept): `counter_items(op, &Counter(other))` = the items of `+data` (`Add`, `Sub`, `RSub` = `other -
  deck`, `And` (keys are *other's*), `Or`, `Xor`); `counter_op` = `Deck(+data)` for plain-value keys.
  `deal.rs` = generator/deal.py: `Deal<O>` (deck + keep tuple; `keep` / `multiply_counts` via the shared
  `keep_tuple::keep` / `keep_tuple::multiply_counts`, which `Pool` now uses too) and `DealSource<O>` (outcomes as
  a `lo..hi` window of the first deck's, like `PoolSource`; `Eq`/`Hash` on deck + keep tuple), a new
  `MultisetSource::Deal` variant, so every evaluator runs over deals unchanged. `math::comb` multiplies out
  (icepool reads `comb_row(n)[k]`, quadratic in the deck size).
- Program layer `program/deck.rs`; `PValue::Deck` / `PValue::Deal`. `program/pool.rs KeepGen {Pool, Deal}`:
  the keep-like nodes (`pool_keep`, `pool_slice`, `pool_highest/lowest/middle`, `pool_mul`, `pool_neg`) and every
  evaluation (`evaluate::run`, catalogue evaluators, `implicit_convert`: a `Deck` raises ValueError like
  `Pool(deck)`) take either; `pool_info` / `pool_pop` / `pool_add` / `pool` stay pool-only (a deal there is
  unimplemented; `Deal + x` is a CompoundKeepGenerator since S4c). `itemize` / `die_arg` / `py_iter` treat a deck as the
  mapping it is (so `Die(deck)`, `Die([deck])` work). A deck or deal result encodes as Python's repr
  (`wire.rs deck_repr`: `{"repr": "Deck: Deck({1: 2})"}`), so the deck-building nodes are fuzz roots.
- Nodes (harness/grammar/deck.py): `deck` (`Deck(outcomes, times)`), `deck_op` (`operator` in `+ - & | ^`, a
  deck on either side; the other side anything `Counter()` takes: deck, die, list, str, None), `deck_mul`
  (`* r* // %` with any operand: non-ints are TypeError), `deck_map`, `deck_pad_to_denominator`, `deck_info`,
  `deck_pop`, `deck_stat` (statistics.py's `STATS`), `deck_sequence` (small literal decks only), `deal` (kind
  "deal", no gen), `deal_info`, `deal_pop`. `pool.py gen_pool(..., deals=True)` (evaluate.py's generators and
  `pool_index`) makes a deal instead of a pool 25% of the time, with the same keep-like operations on top. Quirks
  copied: `deck.sequence(0)` raises UnboundLocalError, a tuple holding Reroll in `Deck(...)` raises only with
  nonzero dups (DIVERGENCES.md); `None + deck` is the deck (`Counter(None)` is empty); `[1] + deck` / `"ab" + deck`
  are the deck's reflected operators (sequences have no numeric slots), but `x ^ deck` is TypeError (no
  `__rxor__`); a non-int `times` element is fine if it is 0 or its argument has no cards, else ValueError.
- Facade: `Deck` holds `icepool_rs.Deck` as `_rs` (`Deck._new_rust`, `_rust_deck()` converts a Python-made deck
  once, `_rust_die()` = its cards as a core die). `Deck.__new__` keeps `contains_again` / `itemize` / the
  single-deck shortcut and calls `creation_args.expand_args_for_deck_rust` (tags as for dice). The operators
  count `Counter(other)` in Python and pass its items to `icepool_rs.deck_counter_op`, then build
  `Deck(dict(items))` (so keys that are dice / Reroll / tuples of dice expand as Python's `Deck(+data)` does).
  `_popped_min/max`, `hash_key` (`(Deck, _rs)`) on the core; base.py's switched `Population` methods test
  `_rust_population(self)` (Die or Deck) instead of `isinstance(self, Die)`, except `zero` and `split` (they build
  `Die._new_rust` results; for a deck they stay upstream's code, whose `Deck(...)` is the core's). `Deck.map` and
  `Deck.sequence` are upstream's code over core calls. `Deal.__init__` = `icepool_rs.deal_new` (checks and keep
  tuple); `Deal._new_rust(deck, rs)` keeps the facade's deck object; `keep` (keep.py), `multiply_counts`,
  `_set_keep_tuple` (`with_keep_tuple`), `denominator`, `hash_key`, `_make_source` on the core; `DealSource`
  (`_new_rust`, `deck` / `keep_tuple` lazily via `__getattr__`) pops, sizes and prefers its order on the core, and
  `evaluator_rust.is_rust_evaluable` accepts it. `MultiDeal`: on the core since S4b (above).
- Facade gap: a `Counter(other)` value that is not an `int` (a dict operand with float counts) raises
  NotImplementedError; Python goes on (and usually raises ValueError in `Deck(...)`).

- **map(repeat), Markov chains, map_and_time on the core (S3b).** Core `map_tools/` = map_tools/: `core_impl.rs`
  `TransitionCache<F>` (F = `FnMut(&Value, usize) -> Result<Next, E>`: the state and an index into the joint
  outcomes of the extra args, so the *caller* keeps the outcome objects and does `star`; the core only gets their
  quantities). `Next::{Reroll, Restart, Break(Option<Target>), Go(Target)}`, `Target::{Outcome, Die, Reroll}`
  (`Break(Reroll)` -> AttributeError, see DIVERGENCES). Callbacks are lazy exactly where Python's are
  (`is_self_loop` stops at the first leaving transition; a state is stepped only when reached), states are
  discovered incrementally. `mod.rs`: `map_repeat` / `map_and_time` (`repeat: Option<u64>`, `None` = `range()`
  TypeError raised *after* the first self-loop die), `final_map`. `markov_chain.rs` `absorbing_markov_chain` ->
  `(die, Option<Fraction>)` (`None` = Restart possible). Count mode's chain (again.rs) now runs on the same cache.
- Order of transient states: Python pops a `set`; the port explores breadth first (FIFO). Results don't depend on
  it (exact solution, simplified die, Fraction), so the elimination skips Python's zero-multiplier row ops (they
  only scale). Termination does: with infinitely many states and one that raises (a walk unbounded below,
  `Break(Reroll)` at the top), Python's set order reaches the raise; a depth-first port walked off forever
  (fixtures/s3b/unbounded-chain-raises). Breadth first steps every state at finite distance, so it raises too.
- Generators that evaluate a subprogram at generation time (pool.py `_die_without_nan`, again.py `_finite_die`,
  statistics.py, `_gen_pool_pop`) run outside the fuzzer's oracle timeout: use `harness.grammar.peek` (gives up
  after 2s with TimeoutError), never `evaluate`. Attempt 1 of S3b hung the fresh-seed fuzz for 16 minutes on a
  `map_repeat` 'inf' over an unbounded process nested in a pool die. The 'inf' / mean_time_to_absorb generators
  now only build finite chains (`markov.py _UNBOUNDED`, `_ANY_STEPS`).
  What could depend on it is refused as `Unsupported`: equal states spelled differently (`1`/`True`/`1.0`, `0.0`/
  `-0.0`) anywhere in an 'inf' chain (`Spellings`). NaN states are `Unsupported` in every mode (Python finds the
  same NaN object again by identity). A process with infinitely many states never finishes, in Python too (the
  fuzzer skips it as "oracle slow").
- New `Error::NotImplemented` (Python `NotImplementedError` raised by icepool itself; `mean_time_to_absorb` with
  Restart). icepool-py maps a core `Error::Type("AttributeError: ...")` to `AttributeError`.
- Program layer `program/map_tools.rs`: nodes `map_repeat` (`f, args, star, repeat`; `repeat` int or 'inf'),
  `map_and_time` (`f, initial, args, star, repeat`), `mean_time_to_absorb` (kind "value", a Fraction),
  `time_to_sum` (`die, target, max_time, dnf`; the step closure is native). Pools / decks among the args: S3 (top).
  New catalogue processes: `cap_sum`, `break_at`, `restart_on`, `walk`, `walk_die`, `collatz`,
  `pair_walk` (a tuple state, star inferred). Fuzz: `--ops map_repeat,map_and_time,mean_time_to_absorb,time_to_sum`
  (self-contained: the generators build their own small int dice).
- Facade: `TransitionCache.__init__` stays Python (`transition_and_star`); `TransitionCache._rust_args()` hands
  `(transition, star, [arg_items(a)...], kwargs)` to `icepool_rs.map_repeat` / `map_and_time` /
  `absorbing_markov_chain` (crates/icepool-py/src/map_tools.rs). The checks on `repeat`, `Die([first_arg])`, the
  early `repeat == 0` and `NotImplementedError` stay in function.py / markov_chain.py. `Die.time_to_sum`,
  `map_function(repeat=)`, and die.py's pool `map(..., repeat=depth - 1)` go through these unchanged. A `repeat`
  beyond u64 saturates (the loop stops at convergence anyway); a non-int with `__index__` works like `range`.

- **Again in both modes (S3a).** Core `again.rs`: `Again::Apply { function, args, is_additive }` (the flag is
  stored, not derived: Python sets it per dunder, and `AgainExpression(operator.add, ...)` made by hand is not
  additive), `Again::again_count` (= `_again_count`: every non-Again argument becomes `0`, so *any* `@` in count
  mode is `int @ int`, a TypeError, even `d6 @ Again`, which upstream's `is_additive` test calls additive),
  generic `step_outcomes` (= `[make_step_outcome(o) ...]`, the facade tupleizes) and `evaluate_agains_using_count`
  on `Die<Value>`: the `map(step, step_die, star=True, repeat=again_count + 1)` chain done the way `TransitionCache`
  does it (transition die of `(TransitionType int, state)` tuples, `is_self_loop` per next state, `(2, None)` for
  Restart, early stop when nothing is DEFAULT, `final_map` -> Reroll), then `finalize` (`n @ die` is the facade's,
  passed as a closure: program `matmul_dice`). Every intermediate die is built with `Die::new` so quantities match.
  Quirk copied: `Reroll` and dice among the outcomes are *non-Again rolls* in the step die (`(zero, 1, -1)`) but are
  left out of `not_again_die`. NaN anywhere in a state is `Unsupported`: Python's tuples holding the same NaN object
  (the zero of a die with inf, shared by every `(zero, 1, -1)`) compare equal by identity; the generator uses
  finite dice (`_finite_die`).
- Program layer: `again::apply` computes `is_additive` like the dunders (`+`: all expression operands additive;
  `x @ expr`: `__rmatmul__`, a die `x` needs `min_outcome() >= 0`, evaluated, so an empty die raises IndexError at
  construction; `expr @ x` and everything else: not additive). `die_with_agains(outcomes, times, again_count,
  again_depth, end)`; both given -> ValueError; depth mode with `Restart` -> ValueError. Nodes: `again_count`
  (`outcomes, count, depth, end`; depth is normally None) and `Restart` (no generator; used by again.py gens).
- Facade (`python/icepool/population/again.py`): `AgainExpression` and both functions' argument juggling stay
  Python (depth check, default end, Restart check, `compute_not_again_die`, `Die([again_end])`); then
  `icepool_rs.evaluate_agains_using_depth/_count` (crates/icepool-py/src/again.rs) read `_func`/`_args`/
  `is_additive` into the core tree over `Arc<Py<PyAny>>` (pyo3 0.29's `Py` is not `Clone`); the expression
  functions, `icepool.Die(objects, times)` and `icepool.tupleize` are host callbacks. A non-int depth/count raises
  TypeError where `range()` would. A bare `AgainExpression()` is not additive: the core's
  `Again::Again { is_additive }` carries the flag (S3). `Die.explode` now runs on the core too
  (map_final is a host callback of `map_simple`). `explode_to_pool`: done in S3c.

- **Formatting, sampling, `==` on the core (S2b).** Core `format.rs` = population/format.py (`format(die,
  type_name, spec, csv_table)` does `Population.format`'s spec parsing and default columns; `table` = headers +
  rows; `csv` writes the `'excel'` dialect itself), over `value/pystr.rs`: exact `py_str` / `py_repr` (Fraction
  `'n/d'`, single-quote str repr, CPython's 4300-digit int limit; a non-ASCII char in a str *repr* is Unsupported:
  printability needs Unicode tables) and `format_float` (`'f'`/`'e'`/`'%'`: Rust's exact formatting rounds ties
  like CPython's dtoa, checked on 800k values; only the exponent spelling differs). `builtins::math_log10` /
  `float_ceil` (log10 is `f64::log10`: only `format_probability_inverse`'s `ceil(log10(int_start))` uses it). `random.rs` = CPython's MT19937 `seed(int)`
  + `randrange` (`_randbelow_with_getrandbits`), for icepool-run/WASM; `Die::sample_at(r)` is the bisect.
  `Die::equals(other, simplify)`, `Die<Value>::hash_key_compare(op)` (tuple order of `(Die, tuple(items))`).
  New `HostObject` hooks: `py_str`, `py_repr` (fallible), `len_kind` (`Mapping` / `Sized` / neither).
- Facade: `Population.format` (csv writer kwargs: the core's table written by `format.write_csv` with Python's
  csv module), `_common_outcome_length`, `sample` (the draw stays `random.randrange` on Python's global, user-seeded
  generator; `icepool_rs.sample_at` bisects), `format_probability_inverse`, `Die.equals` (`icepool_rs.die_equals`)
  and `Die.hash_key` = `(Die, self._rust_die())`: `icepool_rs.Die` now has `__hash__` (core hash, not Python's
  tuple hash: set/dict *iteration order* of dice can differ from the oracle; nothing in icepool depends on it) and
  `__richcmp__` (== core equality, < etc. tuple order). `DieWithTruth` itself is untouched: its data comes from
  `binary_operator` (core) and its truth from `equals` (core).
- Program layer: `PValue::DieWithTruth(die, truth)` (eager; neither half can raise for values). Nodes (harness/
  grammar/format.py): `format` (spec `None` = `str(die)`), `format_probability_inverse` (`args` = `[]` or
  `[int_start]`), `common_outcome_length`, `sample` (`random.seed(seed)` then n samples), `eq`/`ne` (kind
  "die_truth": only `format`/`truth`/`equals`/`hash_key_compare` consume it, so the type name `DieWithTruth` never
  leaks into "die" nodes), `truth` (`bool(x)`), `equals`, `hash_key_compare`. The single-die shortcut of
  `Die([x])` returns a `DieWithTruth` as is (`ops::new_die`). Fuzzing these needs base ops in `--ops` (the
  generators call `g.die()`), e.g. `--ops format,...,hash_key_compare,d,die,weights,die_typed,add,mixture`.

- **Population statistics on the core (S2a).** Core `population.rs` = population/base.py: generic over `Outcome`
  where only order matters (`quantities(Comparison)`, `quantity`, `nearest`, `mode`, `modal_quantity`,
  `quantity_where(mask)`, `group_by(keys)`), `impl Die<Value>` for the arithmetic (probabilities, quantiles, mean /
  variance / sd / standardized_moment / skewness / excess_kurtosis / entropy, KS / CvM, `marginal(Index)`,
  `marginals_len`, covariance / correlation, multiply / divide / modulo_quantities, pad_to / multiply_to_denominator).
  Methods making a new population return Python's `dict` as `Data` (`(outcome, quantity)` `Value` pairs, `None` =
  Python returns `self`): `Die(dict)`'s weight checks stay with the caller (`program/ops.rs weights_die`, the
  facade's `Population._from_rust_data` = `self._new_type(dict(data))`).
- `value/builtins.rs`: CPython 3.13's `sum` with its fast paths (int accumulation in a C long; float Neumaier
  compensation, but ints added to a float total are *not* compensated; once generic, never back), `to_float`
  (`PyFloat_AsDouble`), `try_fraction` (`Fraction(n, d)` or `n / d`), `math_sqrt`, `math_log` (CPython `loghelper`
  incl. the huge-int frexp path, `m_log` specials, `math_1` domain errors), `getitem(Value, &Index)` (tuple/str
  index and slices exactly as `PySlice_Unpack` + `AdjustIndices`), `py_len`. Use them for any new Python-builtin
  arithmetic.
- New `HostObject` hooks, all defaulting to `Error::Unsupported`: `to_float`, `as_rational` (`numbers.Rational` ->
  `Fraction(n.numerator, n.denominator)`), `as_index` (`__index__`), `getitem(&Index)`, `py_len`. `PyHost` implements
  them in crates/icepool-py/src/stats.rs. Facade gap: an `int` subclass item in `sum`'s float path goes the
  generic way (Python treats it as an int: same result unless compensation was pending).
- Facade (python/icepool/population/base.py, `if icepool.backend.RUST and isinstance(self, icepool.Die)`):
  is_empty, min/max_outcome, nearest, quantity, quantity_where / probability_where (the callable is called in the
  facade, a mask goes to Rust), quantities (except `'=='`, the values view), `_denominator`, multiply / divide /
  modulo_quantities, pad_to / multiply_to_denominator, probability(ies), mode, modal_quantity, KS / CvM against a
  Die (a Deck `other` stays Python), quantile(_low/_high) (so median*), mean, variance, standard_deviation (so sd),
  standardized_moment (so skewness / excess_kurtosis), entropy, `_Marginals.__len__`, covariance, correlation,
  `group_by` (key function called back per outcome inside Rust, so an unhashable key raises before the next call).
  `marginals[...]` needs no edit: `operator.getitem` is now native in icepool-py `ops::Unary::GetItem` (Unsupported
  -> Python's getitem for that outcome); `marginals.attr` stays a host callback. A `Deck` runs the same core
  calls since S4a (`_rust_population`). `has_zero_quantities` does not exist at the pinned oracle (only `Counts.has_zero_values`).
- Nodes (harness/grammar/statistics.py): quantities, probabilities, quantity, probability (`args` = 1 or 2
  positionals), nearest, die_stat (`getattr(die, stat)()` for 15 names), entropy (`base` list), die_quantile,
  cdf_distance, marginal, marginal_slice, marginals, group_by, group_by_index, joint_stat, quantities_op,
  pad_to_denominator, multiply_to_denominator, quantity_where, probability_where. `cdf_distance` and tuple dice use
  NaN-free dice (`_stat_die_without_nan`, pool.py `_die_without_nan`): `sorted_union` sorts a set and tuples holding
  the same NaN object compare equal in Python, both identity-dependent.
- Zero quantities: at the pinned oracle no node makes a die with zero quantities (`Die({x: 0})` and mixing drop
  them; only `_pop_min/_pop_max` keep existing ones), so they are effectively untestable through programs.

- **Evaluation driver on the core (S1e).** Under `ICEPOOL_BACKEND=rust`, `MultisetEvaluatorBase.evaluate` keeps
  its argument conversion, `_prepare`, the `_cache` bookkeeping of dungeons (so upstream's evaluator_cache_test.py
  holds unshimmed: it only counts dungeons) and the outer `Die(final_data)`; per prepared evaluation,
  `evaluator/evaluator_rust.py is_rust_evaluable` (`rust_flats` since S4c) (a `MultisetEvaluatorDungeon`/`Quest`, every input one
  `MultisetFreeVariable` + `MultisetGeneratorQuestlet`, every source a `PoolSource` with `_rs`) sends it to
  crates/icepool-py/src/evaluate.rs instead of `Dungeon.evaluate`. `builtin_spec` names the core built-ins
  (`type(evaluator) is` the exact class, its `next_state` arity, only the kwargs it reads with exact types:
  `SumEvaluator()` without map, size, product_of_counts, empty(any: bool), expand(order: Order/int/bool), keep(index:
  None or i64), poker ones, all_counts, count_subset, largest_straight_and_outcome, the seven comparisons; since
  K6e argsort drop/left/right and argsort_grouped (no kwargs, >= 1 input, `_order` an `Order` passed as its int,
  `_limit` None or i64, `_limit_pad` None or exact int) and lexi (2 inputs, kwargs exactly `sort_order` an `Order`
  and `lexi_tuple` a tuple of 5 i64s)) -> `evaluate_builtin`, all in Rust. Anything else (subclasses,
  `SumEvaluator(map)`, wild/reduce counts, argsort with order=1 / an IntEnum / a big limit, wrong arity, extra
  kwargs) -> `evaluate_host`: the core driver with the evaluator's bound methods as host
  callbacks (`HostEvaluator`), states as `Value` (native where possible, `Value::Host` otherwise), `Order` members
  and the `outcomes` tuple passed as Python does, `x in REROLL_TYPES` asked of Python, `UnsupportedOrder` (and
  subclasses) from initial_state/next_state/final_outcome = the core's retry signal (from `extra_outcomes` it
  propagates as is, outside Python's try too), final outcomes returned untouched for `icepool.Die(...)`. Rooms are
  cached per call (decision 5): a dungeon's `ascending_cache`/`descending_cache` stay empty on this path.
- Upstream's Python driver is no longer used under rust (S5c): user sources / operators are host ones, and any
  other unknown preparation raises `NotImplementedError` (see the S5c entry).
- Python's recursion limit on this path: `_depth_budget` = recursion limit - Python stack depth - `_DRIVER_FRAMES`
  (8, calibrated: a fresh-evaluator, cold-die `pool(1)` evaluation raises from the same outcome count as the oracle
  at top level and 20 frames deep). The oracle's real limit also depends on warm `_popped_max` and on the shared
  evaluators' persistent room caches (a shared `sum_evaluator` that saw a superset of rooms before recurses less),
  which the facade does not reproduce.
- Nodes added: `pool_compare` (`cmp` = proper_subset / proper_superset / equal_set / not_equal_set: the evaluator
  called directly, no DieWithTruth), `keep_evaluate` (`keep_evaluator.evaluate(pool, index=)`, any small index or
  None), `evaluate_kw` (catalogue evaluators with `kw=True`; only `scaled_size`, whose unexpected keyword is a
  TypeError raised lazily from initial_state, i.e. not at all when an input has no probability).
- **khepri sweep through the facade** (LOOP-PLAN check 5, third leg: f2f.py on the Rust facade). Build the
  extension first (`uv run maturin develop --release -m crates/icepool-py/Cargo.toml`), or the rebuild inside
  run-facade.sh eats the first program's 20 s timeout and shows up as one failure with reason None:

  ```bash
  ICEPOOL_RUN=$PWD/crates/icepool-py/run-facade.sh uv run python -m harness.core.tool fuzz --ops khepri --seed 70 --count 5000
  ```

  At S1e: 5000/5000 clean in 166 s (the oracle side included). FaceToFace is a user evaluator, so this exercises the host-callback path.

- **Pools on the core (S1d).** Under `ICEPOOL_BACKEND=rust` the facade's `Pool` holds an `icepool_rs.Pool` as
  `_rs` (`Pool._new_rust`, crates/icepool-py/src/pool.rs) and its `PoolSource` an `icepool_rs.PoolSource`
  (`PoolSource._new_rust`). `Pool.__new__` keeps its juggling (`Pool(pool, times)` via `_dice`, the Die/Deck
  check, `itemize`, `implicit_convert_to_die`) and calls `icepool_rs.pool_new(rs_dice, times)` (non-int count:
  TypeError like `(1,) * sum(times)`). Switched: `KeepGenerator.keep` for a `Pool` (keep tuple made and composed
  in Rust; the negative-counts fallback to `MultisetExpression.keep` and the keep evaluator for an int index stay
  Python), `Pool.multiply_counts` (so `*`, `-`), `Pool.additive_union` of pools (`None` from Rust -> Python's
  `KeepGenerator.additive_union`, i.e. CompoundKeepGenerator), `_make_source`, `raw_size`, `denominator`,
  `hash_key` (`(Pool, _rs)`: the Rust object hashes/compares dice + keep tuple), and all of `PoolSource` (`pop`,
  `order_preference`, `size`, `is_resolvable`, `denominator`, `hash_key`). Evaluations over these sources run
  on the core driver since S1e. `_dice`/`_outcomes` (Pool) and `dice`/`_outcomes`/`keep_tuple` (PoolSource) are converted lazily
  by `__getattr__` (facade dice are new `Die._new_rust` objects, equal but not identical to the dice passed in).
  Under rust, `Pool._set_keep_tuple`, `_new_raw`, `_new_from_mapping` and `PoolSource._new_raw`'s `@cache` are
  unused (`clear_cache` is harmless). `make_keep_tuple` / `compose_keep_tuples` stay Python for compound generators.
- Facade gaps (NotImplementedError where Python answers): keep-index sequence elements that are not ints (Python
  keeps floats etc. and sums them in `compose_keep_tuples`), `multiply_counts` by a non-int. Keep counts of any
  size work since K6d. `d(6).pool(2**70)` is OverflowError on both sides.
- Program layer: the `pool` node now does `Pool(pool, times)` (`times == 1` returns the pool, else the mapping
  `{die: count}` of `_dice` through `itemize_mapping`: keep tuple and zero-count dice forgotten). New nodes
  `pool_dict` (`Pool(dict(items), times)`; `program/pool.rs dict` = Python dict merging: first key, last value,
  `True`/`1` one key) and `d_pool`/`z_pool` (`die_sizes`, `mapping` flag = `dict(die_sizes)`; falsy argument ->
  `Pool({Die([0]): 0})`). `ops::itemize` now takes mapping values as `Value`s (`itemize_mapping`). They have no
  gen; `grammar/pool.py _other_pool` puts them into `_base_pool` (25%) so pool_info/pool_pop/... fuzz them.

- **map without repeat on the core (S1c).** Under `ICEPOOL_BACKEND=rust`, core_impl.py `map_simple` keeps
  `transition_and_star` (so `infer_star` and the mapping lambda stay Python's) and calls `icepool_rs.map_simple(
  transition, star, [arg_items(arg) ...], kwargs)`: the Cartesian product (core `iter_cartesian_product` over
  `Bound` objects, so the facade's own outcome objects reach the callable), one host callback per joint outcome,
  `Break` unwrapped (`Break()` = the first outcome before unpacking). It returns `(final_states,
  final_quantities)` and the facade calls `icepool.Die(..., again_*)`: the mixing is the core's like every
  `Die()`, and Again / the single-die shortcut / Restart-as-Reroll stay exactly Python's. `expand.arg_items` was
  lifted out of `iter_cartesian_product` (unchanged) to share it. `cartesian_product(outcome_type=tuple)` over
  dice and plain outcomes (no other expandable, e.g. a multiset expression) is `icepool_rs.tupleize` = core
  `expand::tupleize_dice` (also used by icepool-run's `tupleize`); Deck/Vector/`iter_cartesian_product` itself
  stay Python (S4a/S5, S3b uses it in TransitionCache). `map_and_time` is all repeat: on the core since S3b. The zero-args `map(f)`
  and `MultisetExpression.expand()` of args are untouched Python.
- `creation_args.expand_args_for_die_rust` now expands every arg first and raises AttributeError for a non-Mapping
  subdata (a tuple holding Reroll expands to Reroll itself), as `merge_weights_lcm` does; it was TypeError.
- Program layer: `PValue::Restart` (drops out like Reroll in `die_arg`; only made by catalogue `restart_below`)
  and `PValue::Break(Box<PValue>)` (`Value::None` = `Break()`; only `map` unwraps it, `die_arg` says
  unimplemented, as `Die([Break(..)])` from a zero-arg map would keep a Break outcome). `Func::call_kw` takes
  keyword arguments: only `add_kw` has one (`k`); a name that would bind a positional parameter (`x`, `a`, ...)
  is unimplemented, any other is TypeError. Node `map_kw` (kwargs as `[[{"str": name}, value], ...]`); catalogue
  entries with `kw=True` are only picked by `fn_node(g, kw=True)`.

- **Die methods on the core (S1b).** Switched under `ICEPOOL_BACKEND=rust`: `Die.reroll` (and so `filter`),
  `Population.split` and `zero` for a die (`isinstance(self, Die)` in base.py), `truncate`, `clip`, `simplify`,
  `_popped_min/_popped_max`, `__matmul__` (so `__rmatmul__`, `count`), `Die._lowest_single/_highest_single`, and
  keep.py's `_lowest_single/_highest_single`. Pattern for a selection: the facade runs `_select_outcomes` (Python
  callables and `in` stay Python's, identity semantics included) and passes `[o in outcome_set for o in
  self.outcomes()]`; `icepool_rs` takes the flags. `@` goes through `icepool_core::program::matmul_dice` (shared
  with icepool-run: float/Fraction counts, the `_sum_all` recursion check; a core `Error::Value("RecursionError:
  ...")` is re-raised as `RecursionError`); an `int`-subclass count (IntEnum) is converted to the int first. The
  core's sum cache is per call: the facade die's `_sum_cache` is no longer filled (`_sum_all` itself stays
  upstream's Python, now unused by `@`). Non-int `depth`/`rolls` (`icepool_rs` `exponent`): an int-valued
  non-negative Fraction acts as the int; a float or other Fraction makes Python's `**` give floats, so the result
  is the raise Python ends in (OverflowError / ZeroDivisionError / TypeError, see `reroll` and `die_keep_single`).
  icepool-run still says unimplemented for a float reroll depth (`program/map.rs depth_arg`); port the same
  logic there if a generator ever produces one.
- Left in Python on purpose: `explode` (it is `map` + Again: on the core since S3a), public `icepool.from_cumulative` (its
  outcomes may be dice/Reroll mixed by `Die()`, which is the core's; only the subtraction loop is Python), the
  `Deck` paths of `split`/`zero` (they build the result with `Deck(...)`, on the core since S4a). Grammar nodes added: `zero`, `simplify`, `die_pop` (`(die, quantity)`,
  kind "value"), `from_cumulative` (int cumulative only; cumulative dice are unimplemented in icepool-run).
- Facade gaps (both sides raise, only the type or the empty-die case differ): `truncate`/`clip` hash their bounds
  on entry, so an unhashable bound raises TypeError even on an empty die (Python returns the empty die).

- **Python facade (S1).** `python/icepool` is a copy of the oracle's `src/icepool` (git ls-files, so no caches)
  plus `backend.py`: `ICEPOOL_BACKEND=python` (default) or `rust`; `icepool.backend.RUST` is the `icepool_rs`
  module or `None`, and switched code tests `if icepool.backend.RUST:`. No silent fallback: with `rust`, a failed
  import of the extension fails `import icepool`. Mark every facade edit (`icepool-rs:` in a docstring/comment).
  Switched so far: `Die.__new__` after `itemize` / Again / the single-die shortcut (`creation_args.
  expand_args_for_die_rust`: `expand_arg` stays Python, each subdata is passed tagged by kind), `Die.
  _unary_operator` and `Die.binary_operator`. A die made by the core keeps it as `_rs` (`Die._new_rust`);
  `Die._rust_die()` converts a Python-made die once (`from_counts_items`, trusting its order). The facade's
  `Counts` is built with `Counts._from_sorted_items` (no re-sort). Everything else still runs upstream's Python
  code over those Counts: that is the queue S1b-S5b in tasks.json.
- **`Value::Host`** (core `value/host.rs`): an outcome the core has no variant for. Any operator, comparison or
  `==` with a host object on either side goes to `HostObject` with *both* operands (Python: the real
  `operator.<op>(a, b)`, so reflected operators and `NotImplemented` work); a tuple holding one compares natively
  until the differing element. `Error::Host(HostError)` carries the Python exception through the core unchanged
  (a custom exception class survives). Hash: the host's own hash fed like a number's, so an `IntEnum` merges with
  an equal int; a `StrEnum` / namedtuple does NOT merge with an equal str / plain tuple as Python's dict would
  (Rust hashes str/tuple its own way). Program results never contain a host value.
- **`icepool_rs` (crates/icepool-py)**, pyo3 0.29 (no `extension-module` feature: workspace build/test/clippy
  link fine). `convert.rs`: exact `bool/int/float/Fraction/str/tuple/None` become native values, everything else
  (subclasses, `complex`, user classes; Vector/Symbols are native since S5a) a `PyHost` (hash taken on entry: unhashable raises
  TypeError like the dict insertion). Big ints cross as hex (exempt from Python's int/str digit limit). `ops.rs`:
  the callable the facade passes is matched by identity against `operator.*`, `math.trunc/floor/ceil`, `round`;
  `operator.eq/ne` use `Value ==` unless a host object is inside (then Python's `==`); anything else, or any
  extra args/kwargs, is a host callback per outcome. A core `Error::Unsupported` for a native op (complex
  arithmetic, printf `str %`) calls the Python operator for that outcome, as for host objects; elsewhere
  Unsupported raises `NotImplementedError("icepool_rs: ...")`. Results containing `Reroll`/`Restart` as a key
  raise TypeError like `Counts`.
- **Checking the facade.** `harness/core/upstream_suite.sh` (in the gate) builds with `maturin develop --release`
  into `.venv` and runs upstream's tests with `ICEPOOL_BACKEND=rust`; it needs `harness/upstream_shims/conftest.py`,
  which imports the facade before pytest's `pythonpath = src` (oracle pytest.ini) and harness/__init__.py could
  put the oracle first, and refuses to run otherwise. Not in the gate, run them yourself after touching the
  facade: `ICEPOOL_RUN=$PWD/crates/icepool-py/run-facade.sh uv run python -m harness.core.tool fuzz ...` /
  `fixtures verify` (programs evaluated by the harness's Python grammar with `icepool` = the facade; rebuilds the
  extension when a source is newer), and `uv run python -m pytest crates/icepool-py/tests -q -p no:cacheprovider`
  (host outcomes, custom exceptions, big ints, callables with args: oracle vs facade in two subprocesses; add a
  case there for anything the program language cannot express). At S1: all 2654 fixtures, 26k fuzz programs
  (seeds 11-13, depth 4) and 500 khepri inputs clean through the facade; the upstream suite takes 15 s on rust
  vs 17 s on python.
- Known facade gaps: Python's dict merges the *same* NaN object twice (identity), the core never merges NaNs
  (same gap as the port, see Gotchas); non-int weights with `__index__` (numpy ints) raise TypeError where Python
  raises ValueError later.

- **WASM facade (K8).** `crates/icepool-wasm` (wasm-bindgen only; JSON/BigInt/Reflect are declared as externs
  rather than pulling in js-sys): `run(programJson) -> string` = one icepool-run line; `calculate(engineInput)` =
  `f2f.calculate` (input object in, `{rolls, outcomes:[{player, wounds, chance}]}` out, throws where Python raises;
  `rolls` is a Number up to 2**53-1 and a BigInt beyond, as Pyodide's `to_js` converts a Python int).
  `build.sh [OUT]` = wasm-pack `--target web` and `--target nodejs` into `$CARGO_TARGET_DIR/icepool-wasm/{web,node}`.
  `run-wasm.sh` rebuilds the node package when any core/wasm source is newer, then speaks the icepool-run
  protocol. That package is `--no-opt` (under 1 s once cargo has built the wasm32 target): `wasm-opt -O3` took 6 to
  45 s under load, and the build must fit in the harness's 20 s per-program timeout (a cold cargo build does not:
  build the wasm32 target first, as the long gate does). `--test` and `--bench` use the optimised build; `--test` checks
  `calculate` against the 40 recorded `f2f.calculate` outputs, `--bench` times khepri. A panic is a trap in WASM
  (no unwinding): run-wasm.mjs answers like icepool-run's panic guard and reloads the instance.
- **Float math (B9, 2026-10-17).** Transcendentals (`pow`, `log`, `log10`, `exp`, `sin`, `cos`, `atan2`, `hypot`)
  are Rust's `f64` methods, called directly. Natively they are the platform libm, which is what CPython calls, so
  native and Python builds stay bit-exact. Under WASM they are Rust's own libm and may be 1-2 ULP off (`7.0 **
  -2.0` was 1 ULP off): Alex accepted that, so WASM runs compare floats within 2 ULP (`ICEPOOL_FLOAT_ULPS=2` in
  compare.py and the long gate; the JS fixture suite defaults to 2 too, `ICEPOOL_FLOAT_ULPS=0` makes it exact).
  Until B9 these were line-by-line ports of glibc 2.43 (Arm's pow/log/exp, IBM's sin/cos/atan2, glibc's hypot);
  they are gone, with QUESTIONS.md V1 (their LGPL licence) moot. Don't port libm functions again. Only float
  values (`{"float": ...}` leaves) get the tolerance: a complex is compared by its repr, so a complex power
  through WASM can still mismatch in the last bit (none of the fixtures do today).
- **Measurements (K8)**, on this machine, inputs in `crates/icepool-wasm/bench-cases.json` (heavy = 6+2 vs 6+2,
  both EXP + Continuous: 536-bit denominator, heavier than ANALYSIS.md's 258-bit case):

  | | CPython 3.12 (oracle) | Rust native (icepool-run) | WASM in node 22 (first call / warm) |
  |---|---|---|---|
  | typical, 3 vs 1 | 11.5 ms | all three cases 30 ms, | 12.0 / 1.2 ms (first includes compile) |
  | mid, 4 vs 2 DA | 24.4 ms | process start | 3.7 / 2.5 ms |
  | heavy | 377 ms | included | 53 / 46 ms |

  WASM size: 1,075,543 bytes (381 KB gzipped) after wasm-opt -O3, against Pyodide's 5.6 MB; instantiate 6 ms. The
  release profile is speed-tuned (workspace-wide); `opt-level = "s"` for the wasm build is an option if size matters.

- **khepri engine (K7).** `src/khepri.rs` ports f2f.py natively and typed: dice are `Die<i64>` (`d20 > save` is
  0/1 ints: equal hash/order/sums to Python's bools, and every result outcome is an int anyway), the roll is
  `Die<Roll = [i64; 4]>` (`Outcome` for `[i64; N]` is in outcome.rs), the result `Die<(Player, i64)>` with
  `Player` variants in Python's str order. Each step uses the core op Python uses, so quantities are exact:
  `map` = `Die::new(results, quantities)` (helper `map`), `explode` = core `evaluate_agains_using_depth` with a
  one-op `Again` (`o + Again`), `n @ die` = `Die::matmul(&[(n, 1)])`, `Pool.highest`, `FaceToFace` and a native
  `Sum` evaluator through `evaluator::evaluate`. `engine_values` converts to `Die<Value>` for the `khepri` node
  (special-cased in `program::eval`: its `input` is lazy raw JSON). `Input::from_json` only accepts ints / bools /
  null as khepri sends them; anything else (missing key, float, bool in an int field, negative dice count) is
  `Unsupported`, since Python reads keys lazily and its answer would depend on which keys it reached.
- `khepri::calculate` / `calculate_json` = `f2f.calculate` for the WASM facade (`rolls` as an exact JSON integer,
  it passes 2**53; `chance` via `ratio_to_f64` = `float(Fraction)`). tests/khepri_calculate.rs checks it bit for
  bit against 40 `f2f.calculate` outputs recorded from the oracle (khepri_calculate.json).
- `value/ops.rs` `MAX_INT_BITS` is a `u64` now: `1 << 32` did not compile for wasm32. `cargo check --target
  wasm32-unknown-unknown -p icepool-core --lib` builds.

- **The rest of the built-in evaluators (K6c).** Core: `evaluator/basic.rs MappedSumEvaluator<F>`
  (`SumEvaluator(map)`, `add_to_sum` shared with the plain sum), `poker.rs LargestCountWithWildEvaluator<S>`
  (selectors return the truth value; `is_wild or wild_low(o)` short-circuits) and
  `AllStraightsReduceCountsEvaluator<R>` (Value states exactly as Python's tuples), `evaluator/argsort.rs`
  (`ArgsortEvaluator { tie: Drop/Left/Right, order: Value, limit, limit_pad }`, `ArgsortGroupedEvaluator`; `order`
  is a Value compared with `!=` as Python does), `evaluator/lexi.rs` (`LexiComparisonEvaluator { sort_order,
  lexi_tuple }`: the evaluate() kwargs as fields). Callables are generic closure parameters: the program layer
  (`program/evaluate.rs`) builds them from catalogue `Func`s. A callable returning a die or Reroll inside an
  evaluation is Unsupported (states are Values), so the generators only pick plain-valued functions (`_plain_fn`).
  `basic.rs py_sorted_reverse` = `sorted(reverse=True)`.
- K6c nodes (harness/grammar/evaluate.py): `pool_sum_map`, `pool_largest_count_wild`,
  `pool_all_straights_reduce_counts` (catalogue `mul2` = operator.mul, added on all three sides incl. the JS test
  catalogue), `argsort` / `argsort_grouped` (called on the class: `MultisetExpression.argsort(*args, ...)`),
  `pool_leximin` / `pool_leximax`, `mx_compare` (`a o b` for `< <= > >= == !=`; kind **die_truth** like format.py's
  `eq`: a DieWithTruth must not flow into the "die" nodes) and `mx_rfloordiv` (`other // pool`). To fuzz them:
  `--ops pool_sum_map,...,mx_rfloordiv,d,die,truth` (the pool generators write JSON directly).
- `program/evaluate.rs compare_operator` ports the operator protocol: `<` etc. catch TypeError (from the
  conversion or the evaluation) -> NotImplemented -> the reflected operator of an expression on the right
  (attempted once: reflecting again looped forever, a stack overflow the fuzzer found) -> TypeError. `==` / `!=`
  give `DieWithTruth(data, truth)`, truth = `equals` (`same_expression`, hash keys), an evaluation error kept in
  `DieWithTruthError` (Python evaluates the data lazily); an unconvertible operand (TypeError) gives the identity
  fallback `False` / `True`, but a die raises (`Pool(die)` is a ValueError, not caught). A die on the left of any
  of these (or of `//`) raises TypeError: `Die([pool])` sorts with `bool(pool < pool)`.
- `die.pool()` and `pool_dict` keys now take a DieWithTruth (as its die).

- **Die.keep (K6b).** Nodes `die_keep` (`die.keep(rolls, index)`, index an int, a sequence of counts or None) and
  `die_keep_slice` (`die.keep(rolls, slice(start, stop, step))`; the program language has no slice value).
  `program/pool.rs die_keep` composes `die_pool` + `pool_keep` / `pool_slice` + `evaluation("pool_sum")`, as
  `Die.keep` does; the facade's `Die.keep` is upstream's code over the Rust-backed pool, so it needed nothing.
  Die.middle / icepool.middle were already done (nodes `die_middle`, `middle`). Keep counts beyond i64: done in
  K6d (below). To fuzz a node, `--ops` must also list the nodes its generator uses (`d`, `Ellipsis`, ...):
  `fuzz --ops die_keep` alone tests 0 programs.

- **Multiset evaluation (K6).** Core `evaluator/mod.rs` = multiset_evaluator_base.py: trait `MultisetEvaluator<O>`
  (`State: Clone+Eq+Hash`, `Final`; `initial_state`/`next_state`/`extra_outcomes`/`final_outcome`; `Ok(None)` from
  `next_state` is Reroll; `Final::{Outcome, Reroll, None}`; `Err(Error::UnsupportedOrder)` triggers the retry) and
  `evaluate` / `evaluate_with_depth_check` returning `(final outcome, weight)` in `finalize_evaluation` order. A
  room is `(lo..hi window of all_outcomes, sources, state)`; one fresh memo per attempt (the comment on `Dungeon`
  says why that equals Python's two persistent caches). Only generator inputs: each argument is one
  `MultisetSource` whose count passes straight through; multiset expressions (dungeonlet/questlet trees) came in S4c.
  `order.rs` has `merge_order_preferences`, `OrderReason`, `can_truncate`, `lo_hi_skip`; `PoolSource::
  order_preference` (its `KeepSkip` directions are the reverse of `pool_pop_order`, copied). With two sources
  preferring opposite orders the pop order is `Any` and Python evaluates wrongly: copied, see DIVERGENCES.md.
- Built-ins over `Value` (`evaluator/basic.rs`, `poker.rs`, `keep.rs`, `comparison.rs`): states are Python objects
  as `Value` (`Value::None` = not started), bodies use `Value::binary` / `truthy` / `py_max` / `py_min`, so Python's
  quirks come for free (`max(state or count, count)` with a 0 state, `str * count`, tuple comparison). Program
  layer `program/evaluate.rs` (`run`, `implicit_convert` = `Pool(list)`), nodes in `harness/grammar/evaluate.py`
  (`pool_sum`, `pool_size`, ..., `pool_issubset`, and `evaluate` for named evaluators); `pool_index` (`pool[int]`)
  is in pool.py. The rest (sum(map), wild largest_count, argsort, lexi, the comparison operators) came in K6c.
- Named evaluators: `harness/catalogue/evaluators.py` (Python subclasses) mirrored by
  `program/catalogue_evaluators.rs` (`CatalogueEvaluator` variants, `Final = PValue` so a final outcome may be a
  die; the result goes through `ops::new_die`). To add one: a class with `@evaluator(name, params=, inputs=)` and a
  variant + arity in `CatalogueEvaluator::new`. A fixed-arity `next_state(self, state, order, outcome, count)` given
  more inputs is a TypeError at the first `next_state` (`one_count`).
- Every node that evaluates a pool calls `pool.fresh_caches()` first (PoolSource `_new_raw` cache + the shared
  evaluator instances' `_cache`, all `==`-keyed, so a long-lived oracle leaks spellings between programs). The
  port only caches within one evaluation.
- Python's recursion limit: evaluation recurses once per outcome; RecursionError near 980 outcomes, depending on
  stack depth and on whether the dice's `_popped_max` were computed before (process history).
  `program/evaluate.rs` `check_recursion`: raise from 990 outcomes, unimplemented for 931..989. `consecutive`
  (straights) makes big ranges from e.g. `pow`; `pool_expand`'s generator keeps its pools small for this reason.
- Generators keep NaN out of anything pooled (`pool.py` `_die_without_nan`, asks the oracle): a pool's outcomes are
  `sorted(set(...))` and NaN hashes by identity, so Python's order depends on memory addresses.
- Performance: `Die` caches its hash and its popped min/max dice (`OnceLock`s in the shared `DieData`, like
  Python's cached properties; clippy.toml tells `mutable_key_type` that is fine), `Die` `==` short-cuts on pointer
  equality, and `Num::py_hash` has an `i64` fast path. Before that a 960-outcome evaluation took 16s (Python: 3s).
- `Error::Order` (ConflictingOrderError) and `Error::UnsupportedOrder` are new variants.

- **Pools (K5).** Core: `pool.rs` `Pool<O>` (= `Pool`/`KeepGenerator`: `dice` as `(Die, count)` sorted by the
  die's `hash_key` with `py_sorted`, `outcomes` = `sorted_union` including zero-times dice, `keep_tuple: [i64]`),
  `keep` / `multiply_counts` / `additive_union` (`None` = not one pool), `source()` -> `PoolSource` (outcomes as a
  shared `Arc<[O]>` plus a `lo..hi` window; `Eq`/`Hash` on dice + keep tuple only, as Python's `hash_key`), and
  `enum MultisetSource { Pool }` for K6 to add variants to. `PoolSource::pop(order, outcome)` ports
  `iter_die_pop_min/max` (binomial row from `math::comb_row`), the `itertools.product` order (last die fastest),
  and the skip collapse (all-zero remaining keep tuple -> one empty source yielded last). `keep_tuple.rs`:
  `make_keep_tuple` (all Ellipsis forms, incl. the center-`...` TypeError), `compose_keep_tuples`, pops, and
  `lowest_slice`/`highest_slice`/`middle_slice`/`Slice::canonical`. `keep.rs`: `lowest_single`/`highest_single`
  (free functions); `Die::{lowest_single, highest_single, pop_min, pop_max, quantities_le/ge, quantity_le/ge}`,
  `die::from_cumulative`, `function::sorted_union`. Keep counts are unbounded `Int`s since K6d (see there).
- A pool's repr has an address, so a pool is never a program result: the pool-building nodes (`pool`, `die_pool`,
  `pool_keep`, `pool_slice`, `pool_highest/lowest/middle`, `pool_mul/neg/add`, `Ellipsis`) have **no gen**; they
  are generated inside `pool_info` (public accessors + `_dice`) and `pool_pop` (one pop of `_make_source()`) by
  `grammar/pool.py` `gen_pool`. Any node returning a new kind with a gen becomes a fuzz root: keep that in mind.
- `pool_pop` and every evaluating node call `pool.fresh_caches()` (see K6 above). In `gen_pool` only a pool's last
  keep operation may make counts negative (keeping after that is a `MultisetKeep` expression, ported in S4c and
  generated by multiset.py); `pool_index` pools never do.
- `Pool.__new__` quirks copied: a float/Fraction in `times` raises TypeError (`(1,) * sum(times)`), even `0.0`;
  `Pool(die)` raises ValueError; `Die.pool(-1)` raises (itemize). `middle(keep > size)` keeps only the top roll
  (DIVERGENCES.md).
- `sorted_union` dedupes in first-seen order then sorts; Python sorts the set in hash order. Same result for
  totally ordered outcomes; with NaN or partly incomparable tuples Python's answer depends on set order.

- **`@`, Again, explode (K4).** Core: `Die::matmul(&[(i64, Quantity)])` (= `__matmul__` with this die on the
  right) over the `SumOutcome` trait (`+`, unary `-`, `x * 0`; impls for `Value` and `i64`), with a per-call
  `SumCache` that computes only the sums Python's `_sum_all` would (a `zero()` of mixed types raises only if count
  0 is needed). `Die::zero`, `Die::simplify` live there too. `again.rs`: `Again<F, L>` (closed enum: `Again` or
  `Apply(f, args)`) and generic `evaluate_agains_using_depth`; the program layer instantiates it with
  `AgainOp {Binary, Unary, MatMul}` and `PValue` (`PValue::Again`), so evaluating an expression goes back through
  `ops::binary` / `ops::matmul` and Python's dispatch. Count mode: S3a (above).
- Every operator with a `PValue::Again` operand builds an expression (dice return NotImplemented for it; a
  comparison with the expression on the right reflects; `str % Again` is printf formatting). `ops::die_with_agains`
  is `Die(outcomes, times, again_depth=, again_end=)`: `itemize` now returns `PValue`s and `new_die` converts
  them, so the `die`/`mixture` nodes handle Again too (default depth 1).
- `n @ die` with float counts: integral ones make float factors and so float quantities; `Die(data)` then raises
  unless every float-touched outcome totals zero (ops.rs `matmul`). Fraction counts stay int (`int ** Fraction`
  with an integral exponent is an int). Non-integral counts raise (RecursionError in Python).
- `_sum_all` recursion: Python recurses once per roll not yet cached, raising RecursionError near 985 at top
  level. `check_sum_all_recursion` raises for a step >= 1000, says unimplemented for 800..1000. The `_sum_cache`
  lives on the die object and `icepool.d` is `@cache`d, so in a long-lived oracle process a warm `d(k)` can make
  a deep step succeed: never write fixtures whose result depends on a recursion past ~800, and keep generators
  (again.py) from producing big counts.
- NaN now hashes to a fresh value per call (`Num::py_hash`): Python hashes NaN by identity, and a constant made
  sums of dice with NaN outcomes quadratic (timed out in the fuzzer).

- **map and friends (K3).** Callables are named entries in `harness/catalogue/functions.py` (Python factories,
  e.g. `add_k(k)` returns `lambda x: x + k`) mirrored by `program/catalogue.rs` `Func` variants with the same
  params, evaluated lazily like the closure. Bodies use the program layer's Python operators (`ops::binary`,
  `catalogue::truthy`, `py_eq`), so outcome types behave as in Python for free. To add a function: an `@entry` in
  functions.py (with `params`/`args` generators, `pred=True` if it is a predicate) and a variant + name + arity in
  catalogue.rs. `Func::positional()` is `count_positional_parameters` (drives `infer_star`); `None` = a mapping.
- `PValue` grew `Reroll`, `Tuple(Vec<PValue>)` (only for tuples holding non-values: build with `PValue::tuple`)
  and `Func`. `ops::die_arg` is `expand_arg`: `Reroll` -> `DieArg::Reroll`, `PValue::Tuple` -> `map::tupleize`
  (Cartesian product into a die of tuples; a tuple holding Reroll raises, see DIVERGENCES.md).
- `program/map.rs`: `map` (= `map_simple`, no repeat/Again/Break: S3), `_select_outcomes` (`select`) and
  membership (`contains`) for reroll/filter/split; the core does `Die::{reroll, split, clip, truncate}` from a
  per-outcome mask. `expand::iter_cartesian_product` is generic over `(item, quantity)` lists.
- The `fn` node's `name` is lazy (raw JSON string); `program::eval` special-cases it. Add other lazy args there.
- A die has no truth value in Python (`if die:` raises), so `catalogue::truthy(Die)` raises; `==` with a die
  (DieWithTruth) is not modelled yet and returns Unimplemented.

- **Operators (K2).** `value/ops.rs`: `Value::binary(BinOp, &Value)`, `Value::unary(UnOp)`, `Value::round(&ndigits)`
  are Python's `operator.*` on outcomes (numeric tower per CPython/fractions.py, str/tuple concat and repeat).
  `Die::binary_operator` / `unary_operator` (die.rs) are generic over a closure, merge results in an `OrderedMerge`
  (dict order, first key spelling kept), drop zero quantities (what `Die(dict)` does) and sort with `Counts::new`.
  The program layer (`program/ops.rs` `binary`) does Python's dispatch: die on the left -> `Die.__op__`; die only
  on the right -> `__rop__` with the order kept (`3 - die` = `Die([3]) - die`), but comparisons reflect
  (`3 < die` = `die > 3`); scalar with scalar -> plain value result.
- **`pysort.rs` is a faithful port of CPython 3.13 `list.sort`** (count_run, binary insertion, powersort merges,
  galloping). `counts::py_sorted` re-exports it. Needed because NaN outcomes (`inf * 0`, `inf - inf`, `inf // x`)
  are unordered and land wherever the exact comparison sequence puts them. Keep it exact; do not "optimise" it.
- `Value::Complex(re, im)`: from `negative ** fractional` (CPython `_Py_c_pow`) and arithmetic on it (V1,
  `value/complex.rs`). Never survives in a die (unorderable: `Counts` raises, even for a single outcome), unless its
  quantity is 0 and it drops out first, so it only reaches results as a scalar.
- `Error::Unsupported(what)`: Python would answer but the port does not cover it yet (`str %`
  with `%` conversion specifiers). The runner reports it as `unimplemented`, never as a raise.
- Size limits where Python would compute for minutes or hit MemoryError: `**` and `<<` results above 2**32 bits
  and sequence repetition above 2**28 elements raise `OverflowError` (`MAX_INT_BITS`, `MAX_REPEAT` in ops.rs).
  `0`, `1`, `-1` to any power and `0 << n` stay exact.
- `number.rs` `ratio_to_f64` / `int_to_f64`: correctly rounded int/int -> float (CPython `long_true_divide`),
  `0 / -3` is `-0.0`. Also used for `float(int)` and `float(Fraction)`.

- **Module layout (K1)** of `crates/icepool-core/src/`, mirroring icepool's own files so ports stay findable:
  - `outcome.rs`: `trait Outcome` (Clone + Hash + Eq + `py_lt -> Result<bool>` + `check_key`). `Eq`/`Hash` are
    Python's `==`/`hash`. The core is generic over it; `i64` implements it too (typed fast path later).
  - `value/`: `enum Value { None, Bool, Int(IBig), Float(f64), Fraction(RBig), Str(Arc<str>), Tuple(Arc<[Value]>) }`,
    the dynamic outcome both bindings use. `number.rs`: exact int/float/Fraction comparison and CPython's numeric
    hash (mod 2**61-1, so equal numbers of any type hash alike). `repr.rs`: Python `repr(float)`. Vector,
    Symbols (S5a) and Host are variants here; add Die-as-value the same way.
  - `error.rs`: `Error::{Type, Value, ZeroDivision, Overflow}` named after the Python exception (facade re-raises).
  - `counts.rs`: `Counts<O>` (sorted items, zeros kept), `py_sorted` (stable merge sort using only `<`, stops on the
    first raising comparison), `OrderedMerge` (= `defaultdict(int)`: insertion order, first key spelling kept).
  - `math.rs`: `weighted_lcm`, `lcm`. `creation_args.rs`: `DieArg<O> { Outcome, Die }` and `merge_weights_lcm`
    (add `Reroll`, tuples-of-dice/`cartesian_product` as variants in K3). `die.rs`: `Die<O>` = `Arc<Counts<O>>`,
    structural `Eq`/`Hash` (= `hash_key`), `Die::new(args, times)`. `function.rs`: `d`.
  - `program/`: the Python-dynamic layer in miniature. `mod.rs` evaluates the tree (`PValue { Value, Die, List }`,
    `PErr { Raise(Error), Unimplemented }`), `wire.rs` = encode.py, `ops.rs` = one fn per grammar node. Python
    argument juggling (itemize, dict comprehension, `times` sequences, non-int weights) lives in `ops.rs`, not in
    the core; the core takes `Vec<DieArg>` + `Vec<Quantity>`.
- **Big integers: dashu (`dashu-int`, `dashu-ratio`, `dashu-base` for `Gcd`/`UnsignedAbs`).** `UBig`/`IBig` store up
  to 128 bits inline, so small quantities never allocate (num-bigint always heap-allocates); `RBig` is the
  normalised `Fraction`. `Quantity = UBig` (quantities are never negative in icepool).
- Error results carry an extra `"message"` key for debugging; the comparators ignore it.

## Gotchas

- **Surviving mutants (task M202610142332).** Fixtures only check *that* both sides raised (any type, any
  message) and never match `Unsupported`. So mutants that only change an error's type or message, or a resource
  limit, need **unit tests**: `program/mod.rs` `tests::raised(program)` returns `run`'s `(error, message)` for
  that. To reproduce one mutant locally: `cargo mutants -p icepool-core --file <path> --re '<escaped
  description>' --output /tmp/x` (~45 s per mutant: the fixtures run in debug). A task's line numbers may have
  moved; `--list` shows the current ones. Kill fixtures: fixtures/m202610142332/. Left alive on purpose,
  because they are equivalent (no input tells them apart): `function.rs` `pointwise` `q < b` -> `<=` and `q > b`
  -> `>=` (which of two equal ints is kept); `consecutive` `stop > start` -> `>=` (when they are equal the
  difference is 0); `one_hot` `sides > 4096` -> `>=` (only `one_hot(4096)`, a 16M-bool die, tells them apart);
  `outcome.rs` default `py_eq` -> `Ok(false)` (only the `i64` / array / khepri outcomes use it, and their
  equal keys are identical, so the merge is the same); `PartialEq for Value` deleting the `(Vector | Symbols,
  _)` arm (the fallthrough arms already say unequal); `MultisetFunction::next_state` / `next_state_counts` ->
  `Ok(None)` (the driver calls `next_state_tree`; flat counts cannot address a call tree).
- **Surviving mutants (task M202610151225).** Unit tests now cover the facade-only paths (`HostSource`,
  `OrderReason::from_int`, `one_hot_items` positions) and resource limits (`operator::repeat` at 2^24 elements,
  `mean_time_to_sum` of a huge target, `explode_counts` depths). Tuple counts reach `next_state_tuple` through
  `union(multi_deal)` (one child: the tuple passes) under `tuple_op_evaluate` / `record`; keep the other side
  deterministic (a pool of a one-outcome die, or a `keep_outcomes([k])` filter that is 0 before the tuple
  arrives), else `record` mixes int and tuple counts and the final die's sort raises. Lexi's reverse order: the
  driver pops descending by default and evaluates backward, so `next_state` sees ascending; leximax (sort
  descending) of a plain pool runs reversed, leximin only with a pool preferring ascending pops (`pool_lowest`).
  Kill fixtures: fixtures/m202610151225/. Left alive on purpose, equivalent: `poker.rs`
  `LargestCountWithWildEvaluator::initial_state` -> `Ok(Default::default())` (that is `None`); `argsort.rs`
  `count > max_count` / `count > tie_count` -> `>=` (on a tie the tie count ends equal to the max either way, so
  nothing is appended until a larger count resets the index) and every `append_count > 0` -> `>=` (appending 0
  is a no-op, and the `limit` break only fires when nothing more fits); `to_pool.rs` `explode_counts` guard `depth
  < -1` -> `<= -1` / `< 1` (the guard only sees depths below -1 or above `u32::MAX`).
- **Surviving mutants (task M202610160934).** Kill fixtures: fixtures/m202610160934/ (all_straights with a
  negative count after runs started, via `pool + -pool`; `additive_union` of two `union(multi_deal)`s, whose
  sizes are unknown so the tuple counts concatenate instead of raising at `0 + tuple`; `expr * Again` vs `Again *
  expr` argument order, seen by comparing with `Again * 1` / `1 * Again`; a die on the left of a multiset
  operator). Unit tests: `Int` hash/Debug, `list_len` limits, `CatalogueEvaluator`'s `int` forms (the driver only
  calls the `*_counts` forms, so nothing else reaches them), error messages of `die_sequence` / `expr * die` /
  `die + expr` (a fixture cannot see that the die is never made a pool: `Pool(die)` raises too), and
  the NaN-in-Again-hash_key `Unimplemented`. That check now needs a NaN on **both** sides (`&&`, was `||`): a NaN
  on one side cannot be identical to anything on the other, so `==` decides, as the oracle does. Two NaN leaves
  in a program are distinct objects (the oracle says False), but the port keeps declining both-sided NaN since it
  cannot see identity. Left alive, equivalent: every `int.rs` `<`/`>` -> `<=`/`>=` against zero on a `Big` (a big
  value is never 0) and in `floor_divmod` (`r != 0` and `b != 0` there); `keep.rs` `order.value() > 0` -> `>=`
  (the driver merges in `Descending`, so `Order::Any` never reaches an evaluator); `poker.rs` all_straights
  `count < len` -> `<=` (at `count == len` both branches give the same runs: ended runs are always sorted
  already); `binary.rs` `reduce` without its `(Int, Int)` arm (only called when some count is a tuple, and any
  int beside a tuple is the same TypeError either way); `collection.rs` `die_sequence` `i < 0` -> `<= 0`;
  `MultisetFunction::next_state` -> `Ok(None)` (as before).
- **Tuple comparison uses Python's `==` to find the first difference** (`Value::first_difference`, shared by
  `py_lt`, `ops::compare`, `Value::binary`): `Symbols('ab') == ('a', 'b')`, so `(Symbols('ab'),)` and
  `(('a', 'b'),)` compare by length, as Python does (they used to raise). Elements equal as dict keys skip
  `py_eq`, which also stands in for Python's identity shortcut. Fixtures: fixtures/m202610142332/tuple-compare-by-eq.
- **Oracle coverage report (T1).** `uv run --with coverage python -m harness.tools.oracle_coverage` (~2 min) runs
  every fixture plus the gate's fixed-seed fuzz programs (seed 1 x 2000, seed 11 x 10000 at depth 4, khepri seed 7 x
  150; same generation as `tool fuzz`) through the oracle under coverage.py with branches, and rewrites
  `harness-reports/coverage.md` (generated: never edit by hand, rerun and commit it after adding nodes). `--fuzz
  SEED:COUNT[:DEPTH]` / `--fuzz khepri:SEED:COUNT` pick other programs, `--json F` saves the raw data and `--from-json
  F` re-renders without rerunning. coverage.py comes from `--with coverage` (pyproject.toml is protected). At T1: 90%
  of oracle statements, 83% of branches. Triage of the gaps: the biggest are features with a todo task already (K6c:
  argsort, leximin/leximax, comparisons of expressions, largest_count wild, all_straights_reduce_counts, sum(map=);
  K6b: Die.keep; S3c: explode_to_pool / reroll_to_pool / map_to_pool; S4e: weightless; S5c: to_one_hot, remove). Dead
  in the oracle (nothing calls them): order.py `pool_pop_order` and `Order.merge`, the module-level
  `cartesian_product` / `iter_cartesian_product` / `tupleize` / `vectorize` in collection/vector.py (the public ones
  are in expand.py / function.py), Vector.reverse_binary_operator with a Vector. Gaps in ported features became tasks
  T1a (Again over every operator), T1b (multiset operators with reflected / non-multiset operands), T1c (str/repr of
  expressions), T1d (Die/Population branches), T1e (Die method forms of ported functions).

- **Long gate L202610141124 (task P1 fixed here too).** (1) `Die` caches what Python caches: `position` (an
  `FxHashMap` standing in for the dict behind `outcome in self`; the first of equal outcomes wins, and NaN is never
  found, as before), `cumulative_quantities` (`_cumulative_quantities`) and `denominator` (`_denominator`). Without
  them `quantity('<=', x)` per outcome was quadratic: `cdf_distance(d6, 3 @ (d12 / d12))` (41k float outcomes) took
  47 s. New per-outcome queries on `Die` should use these, not scans. (2) The fresh-seed long fuzz died silently
  with exit 137 (OOM kill): `icepool.math.comb` caches every Pascal row up to n in the process-wide
  `comb_row_cache`, so a deal from a deck with `10**20` copies (the `weights` leaf) grew it for 3 s per program and
  it never shrank. `deck.py` `_gen_modest_deck` now re-draws decks over 200 cards for deals and multi-deals; route
  any new dealing generator through it. (3) Seed 15's failure was a traceback: `OracleTimeout` raised *at*
  `signal.alarm(0)` in tool.py's `finally` (the alarm landed after the try body ended, e.g. a long C call finishing
  right at 3 s). A race in the protected harness; it didn't reproduce. If it recurs, it is not the port.
  (4) The (since removed) `value/pow.rs` overflowed in debug builds only, so cargo-mutants refused to run ("cargo
  test failed in an unmutated tree"). Run `cargo test -p icepool-core --lib` in debug too.
- Files copied from the oracle checkout are read-only on disk (`chmod -R u+w python` once; git ignores the mode).
  Never run Python with the cwd inside `python/icepool`: its `operator/` package shadows the stdlib's.
- The upstream suite passing is not proof a switched path is used: check with `ICEPOOL_BACKEND=rust` that dice
  carry `_rs` (e.g. `type((d(6) + 1)._rs)`).
- **`cargo fmt --all` reformats the protected `crates/icepool-core/tests/fixtures.rs`.** Run it, then
  `git checkout crates/icepool-core/tests/fixtures.rs` (the guard rejects any change there).
- A shell-safety hook sometimes blocks long `python3 - <<'EOF' ... EOF` heredocs that it misparses as `rm -r`.
  Write the script to /tmp and run it instead.
- `harness/grammar/construction.py` adds `die_typed` (typed outcomes: bool/float/Fraction/str/tuple, sometimes mixed
  so it raises) and `die_times` (`Die(outcomes, times)` with nested dice). Both return "die", so once arithmetic
  nodes are ported the fuzzer feeds them typed and mixed dice: expect float/Fraction/str results there.
- Python quirks copied in `ops.rs` (all verified against the oracle, fixtures in fixtures/k1/):
  - `Die({x: 0.0})` is an empty die, `Die({x: 2.0})` raises: a non-int weight survives only if it is 0 or its
    argument is an empty die (it never reaches `math.gcd`). Negative or non-numeric weights raise.
  - `Die([die], times=[1.0])` / `Die({die: True})` return `die` itself: the single-die shortcut tests `== 1`.
  - `Die(die)` treats the die as a mapping (zero quantities drop out); `Die("abc")` is a die of characters;
    `Die((3, 1))` a die of 3 and 1; `times` may be any same-length sequence, even a str or a die.
  - `{o: q}` with repeated keys (`True`, then `1`) keeps the first key and the last quantity.
  - Tuples may contain `None` (`Die([(None, 1)])` works); a bare `None` outcome raises.
  - Merged numeric outcomes keep the first spelling: `Die([True, 1])` is `{True: 2}`, `Die([1, True])` `{1: 2}`.
- Float NaN: the port treats NaN as unequal to everything, including itself, and sorts exactly like CPython
  (pysort.rs), so NaN outcomes from arithmetic match the oracle. Remaining gap: Python's `==` inside tuple
  comparison short-circuits on object *identity*, so the *same* NaN object in two tuple outcomes compares equal
  there. Arithmetic never puts NaN in tuples and the generators never produce NaN literals. Same gap in `map`:
  Python's result dict merges the *same* NaN object returned twice (`map(max2, Die([nan]), d2)` is `{nan: 2}`,
  the port gives two NaN outcomes). Only reachable with NaN dice, which only arithmetic on inf makes.
- `str % die` is NOT a die: `str.__mod__` runs first and a die is a Mapping, so it is printf formatting returning
  the str (when it has no `%`). `tuple % die`, `int % die` etc. do reach `Die.__rmod__`.
- `float_repr` (K1) used Rust's shortest digits, which break exact ties upwards; Python's dtoa takes the even
  digit (`714285714285714.2`, `2.9802322387695312e-08`). `repr.rs` `break_tie_to_even` fixes it; checked against
  Python on 100k random floats.
- Operator generators: `operators.py` `operand(g, depth)` / `scalar(g)` (die, int node, or typed scalar incl.
  float/Fraction/bool/str/tuple/None) are shared with basic.py's `_binary`. `pow`/`lshift`/`rshift` use a small
  right operand (`_small`) so programs stay cheap.
- The oracle drops zero quantities passed to `Die({...})` at construction: `Die({1: 0, 3: 5})` has one outcome.
  Zero quantities still survive when mixing a die that has them (`Die([X, 3])` keeps X's zeros; `Die([X])` is X).
- L202610142332 (long gate red):
  - `cargo test`'s "status.json: NotFound" came from the shared `CARGO_TARGET_DIR`: cargo names a path package's
    artifacts without its location and checks source mtimes by relative path, so `fixtures-<hash>` built in one
    worktree (its `env!("CARGO_MANIFEST_DIR")` baked in) was reused from another, which then read the first
    worktree's fixtures or failed once it was deleted. `crates/icepool-core/build.rs` ties the fingerprint to `PWD`
    (gate.sh runs cargo from the checkout root) and this checkout's own path. Each worktree switch now rebuilds
    icepool-core (~20s).
  - The silent "fuzz: fresh seed: FAILED" with no output was the oracle being OOM-killed (exit 137): a straight
    evaluation over a pool holding 10**16 (`stochastic_round(1e16)`, typed floats) makes Python build
    `consecutive()` over that range, and SIGALRM can't interrupt the allocation. Generators now ask
    `pool.py` `huge_range` before choosing a straight (`straight_safe_method`; evaluate.py's straight nodes regenerate
    the pool). If a fuzz run dies without printing, look for exit 137 / huge allocations first; run one seed under
    `/usr/bin/time -f %M`, or the oracle with `resource.setrlimit(RLIMIT_AS)` to catch a MemoryError.
  - `dz_attr`'s free-form names are capped at two digits: a pooled `z961` lands in the recursion-limit band where
    the port says unimplemented (evaluate.rs `check_recursion`).
    `pool_expand`'s generator also peeks its result and retries over 500 outcomes (two d11s + a 5-outcome die,
    each twice, made 990).
  - `sample` of a plain value: AttributeError (a `Symbols` answers `.sample` with a sub-`Symbols`, so it's
    "not callable"), but `()` when `n <= 0` because the generator never runs. Catalogue `first` now uses
    `builtins::getitem` (Vectors index like tuples).
  - Not fixed (not a bug): `die_stat` mean of tuple outcomes multiplies the first tuple by its quantity before
    `0 + tuple` raises, like Python. With quantities ~10**7 that takes ~2.4 GB in the port (Value is wider than a
    PyObject pointer). Under the 20s port timeout and well within memory today.

- **P1 (cdf_distance over `n @ (float die)`).** Most of it was done by L202610141124 (Die caches `position` /
  cumulative quantities / denominator; fixtures/p1/cdf-distance-large.jsonl). This pass found the rest:
  `Num::py_hash` of a finite float went through `RBig::try_from(f)` plus a modular inverse, and hashing was ~75% of
  `3 @ (d12 / d12)` (232k dict inserts of float sums). `value/number.rs` `hash_finite_float` now works from the bits
  (`mant` rotated by `e mod 61`, same value as CPython's `_Py_HashDouble`; unit-tested against `hash_rational` and
  CPython). Each fixture now runs in ~0.2 s (was 0.6 s, and 47 s originally), against ~1 s for the oracle. What's
  left is `py_sorted` of 41k floats (~30 ms) and the dict merge. Profiling tip: `perf` isn't allowed on this
  machine; time the pieces from a throwaway crate in /tmp that depends on `crates/icepool-core` by path.

- **T1a: Again over every operator.** `again.py _again_expr` now builds every binary operator (add ... xor, the four
  comparisons, `==` / `!=`), on either side of a small int or die, `**` / `<<` / `>>` (small right operand from
  operators.py `_small`; with the expression as the exponent / shift count it is wrapped in `% k` first, since an
  again die's outcomes can be huge), the four unary ones, `@` both ways. Node `again_truth` = `bool(expr)`.
  `weights` and `pool_dict` generators sometimes use an Again expression as a dict key.
- Port: `AgainOp::Eq { ne, truth }` (program/again.rs) holds `truth_value`, the only expression with one;
  `again::eq` builds it (`x == expr` reflects to `expr.__eq__(x)`), truth = `hash_key_eq` between two expressions
  (functions by identity, args as tuple `==`: lengths, then element by element, an expression vs anything else
  unequal), else `False` for `==`. NaN in a compared argument is unimplemented (tuple `==` short-circuits on object
  identity). `truthy(Again)` = `again::truth_value` (ValueError unless from `==`/`!=`). A DieWithTruth (what an
  evaluated `==` expression gives) is a plain die as an operand of `binary` / `unary` / `matmul` (`plain_die`).
  Dict keys: Python hashes an expression by `hash_key`, so it only merges with an expression of equal hash_key:
  pool.rs `dict` (now `pub(super)`) compares one by one with `hash_key_eq`; `weights` takes that dict when a key
  is an expression and `weights_die` then rolls again (depth 1, default end) as `Die.__new__` does after itemize.
- Dead in the oracle (confirmed: Python only calls a reflected method when the forward one returns
  NotImplemented, and AgainExpression's forward operators never do): again.py l.106 (`__radd__` with an
  expression `other`: `expr + expr` is `__add__`), l.186 (`__rmatmul__` likewise), die.py l.763
  (`Die.__rmatmul__` with an expression: `expr @ die` is `AgainExpression.__matmul__`) and l.1272-1404 (every
  reflected `Die.__rop__`'s Again check). `implicit_convert_to_die` l.38 is reached by `Pool([expr])` /
  `Pool({expr: n})` (fixtures/t1a/again-dict-keys.jsonl).
- JS suite: `again_truth` has an idiomatic route (fixtures.mjs, `ip.truth(x)`); node-args.json regenerated.
  fixtures/t1a pass through JS.
- **Never run a bare `cargo fmt`**: the tree is not rustfmt-clean, so it rewrites dozens of files, including the
  protected `crates/icepool-core/tests/fixtures.rs` (the guard rejects that). Format only your own files
  (`rustfmt --edition 2021 <file>`), and run `harness/core/guard.sh $(git merge-base HEAD main) HEAD` before stopping.
- **T1b: multiset operators with non-multiset and reflected operands.** Generators (multiset.py): `mx_binary` now
  has a tuple on the left now and then, and `_non_multiset` operands (int, True, float, None, a die) on either
  side; `mx_counts` has `o = "rmul"` (`n * expr`) and a non-int `n` (float, str, list, None, die) for `*` / `%`;
  `_keep` makes two-Ellipsis indexes and non-sequence ones (None, 1.5, a small `d`); `mf_call` / `mf_call_kw`
  sometimes pass no arguments. New node `straight_and_outcome_evaluate(priority, arg)` =
  `icepool.evaluator.LargestStraightAndOutcomeEvaluator(priority).evaluate(arg)` (its constructor's ValueError).
- **Again is not a fuzz operand of multiset operators**, only in fixtures/t1b: `pool + Again` (and `expr * Again`,
  `Again ^ pool`...) is an AgainExpression over the expression, and a node that passes a reflected comparison
  through (`mx_compare`: `pool < that` is `AgainExpression.__gt__`) returns it as the program result, whose repr
  holds an address. The port builds the AgainExpression (`ops::binary`), `implicit_convert` of one is TypeError.
- Port (program/expression.rs): `binary_op` with a number / None on the left is TypeError (both sides return
  NotImplemented), with a die on the left TypeError ("Items do not appear to be sortable": `Die.__op__` makes
  `Die([expr])`, and an expression's `<` gives a die without a truth value), with Again on either side
  `ops::binary`. `not_int` is `*` / `%` with a non-int: Again builds the expression, a die TypeError, anything
  else TypeError. Core `MultisetKeepIndex::Invalid(type name)`: a non-`Sequence` index of a `MultisetKeep`
  raises TypeError only in `_initial_state` (operator/keep.py l.124); the facade builds it too (`keep_index_attr`).
  A str (or Vector) index of a MultisetKeep stays unimplemented (a str is a Sequence: ValueError when evaluated).
- Unreachable through the public API (confirmed against the oracle): multiset_expression.py l.135
  (`MultisetExpression._make_param` with a star index: `star=True` takes `len()` of the first argument first, and
  only multiset *tuple* expressions have `__len__`, so a single multiset is TypeError "has no len()" before it);
  evaluator/keep.py l.45 (`KeepEvaluator.final_outcome` with a `None` final state: `initial_state` always returns
  a tuple; an empty multiset reaches l.47 "ended 1 element(s) short").
- fixtures/t1b: edge-cases (350 hand-picked: every path above, max_pair under the wrong forced order / with a
  right side of unknown size, MultisetMixture.multiply_counts on non-keepable inners, no-argument
  @multiset_function), again-operand-fuzz (6 fuzz failures fixed: an AgainExpression as an evaluator argument;
  the shrinker reduced them to unrelated subprograms, so the originals), keep-against-order (slices with mixed-sign
  bounds and `pool[i]` evaluated against their end over an expression of known size: operator/keep.py l.101-102,
  evaluator/keep.py l.25, covered before only by chance). JS: node-args.json regenerated; `rmul` and the new node
  go through `call()`.
- **T1d: Die and Population branches.** New nodes (all fuzz clean natively, through the facade, and fixtures/t1d
  through JS): `marginal_attr` = `getattr(die.marginals, name)` and `group_by_attr` = `tuple(getattr(die.group_by,
  name).items())` (statistics.py; core `Die::marginal_attr`, `population::check_public_attr` = the `key[0] == '_'`
  guard, IndexError for `''`; `value::builtins::getattr` models only `real` / `imag` / `numerator` / `denominator`,
  which each type has or lacks, plus `Symbols.__getattr__` = `s[name]`; any other name is `Unsupported`, since it may
  be a bound method), `equals_self` = `x.equals(x)` (format.py; `True` for every die / deck / expression / generator /
  Again, which all check `self is other` first: also allowed to take a `DieWithTruthError`, like `truth`, in
  program/mod.rs). `from_cumulative` now takes cumulative dice (`harmonize`, now `pub(super)`, then `quantity('!=',
  False)`); an int first and a die later raises (the die difference has no truth value in `Die()`'s `times < 0`
  check). Catalogue `builtin_max` (`max` itself: `infer_star` raises `InferStarError`, a ValueError, so every
  `Func::infer_star` caller now propagates a `Result`; called, it is `max(iterable)` / `max(*args)`). Generators:
  `group_by` sometimes gets a multi-parameter callable over a die of tuples (base.py l.882), `die_highest` /
  `die_lowest` sometimes 0 rolls (which still never reach die.py l.929 / l.966, see below).
- **Unreachable through the public API (T1d, confirmed against the oracle):** collection/counts.py l.46 / l.48 /
  l.55 (`Counts` only gets the items of a dict built by `merge_weights_lcm` / `merge_duplicates`, after `expand_arg`
  dropped Reroll / Restart; a `None` key never sorts against anything, even alone: `bool(None < None)`; a repeated key
  would already have merged in the dict; `Counts` is not exported); creation_args.py l.93 / l.115 (negative weights /
  duplicates: `itemize` raises first) and l.123 (a zero-quantity outcome of a population argument: no public path
  makes a die or deck with zero quantities, see the Zero quantities note); typing.py l.67 (`hash_key is None`: only
  internal dungeons); order.py l.111 (`ConflictingOrderError`: no source ever prefers an order with `Mandatory`; a
  conflict at `PoolComposition` gives `(Any, Mandatory)`, which the next merge skips because the order is Any), l.152
  (`can_truncate` over `unique_dice()`, a set, so no two of its dice are `equals`), l.195 (`'Should not be
  reached.'`); die.py l.929 / l.966 (`_lowest_single` / `_highest_single` with 0 rolls: the single-die
  path needs `canonical_slice(index, rolls)` to be `[0:1]` / `[rolls-1:rolls]`, and it clamps into `[0, rolls]`, so
  with 0 rolls it is `[0:0]` and the pool path sums an empty pool; brute-forced over 0, 0.0, False, Fraction(0) and
  negative rolls); die.py `Die.filter` 350->352 (a coverage.py artifact: the starred branch's lines run, the arc into the
  blank line after `if star:` is never recorded). Reached now: order.py l.104 (equal reasons after a conflict:
  fixtures/t1d `evaluate record` over a Descending-KeepSkip pool, an Ascending-KeepSkip one and a
  PoolComposition one). base.py 787->789 and l.412-414 (`remove`), l.81 were already covered when T1d started (later
  tasks' generators).
- JS: node-args.json regenerated (it also lacked T1c's `mx_str`, `break_str`); the new nodes go through `call()`.

- **T1e: method forms of ported Die functions.** Most was already there when T1e started: nodes `die_sequence`
  (collection.rs, `cartesian_product` of the die with itself), `die_stochastic_round` (function.rs, a `map` of
  `Func::StochasticRound`) and `mean_time_to_sum` (core `Die::mean_time_to_sum` with the caller's cache vector; the
  facade's `_mean_time_to_sum_cache` is a Python list passed to `icepool_rs.mean_time_to_sum`, done by S5c). New:
  node `die_mean_time_to_absorb` (markov.py; program/map_tools.rs `die_mean_time_to_absorb` = `the_die` then the free
  function's `mean_time_to_absorb`, as die.py l.535 does). Its generator reuses `_process(finite=True)` and wraps a
  non-die start state (an int, pair_walk's tuple) into `Die([state])`, since `self` must be a die. JS: route via
  `die.meanTimeToAbsorb` in fixtures.mjs `IDIOMATIC`; node-args.json regenerated. Fixtures: t1e/die-mean-time-to-absorb.
  A facade fuzz run once reported `FAIL None` on its very first program while run-facade.sh was rebuilding the
  extension (maturin); the same seed is clean afterwards: a timeout from the rebuild, not a port bug.
- **Population statistics take a deck (T1e).** Adding a node shifts the fixed fuzz seeds' streams, and seed 1 then
  minimised a failure to `die_stat` over a `deck`: in Python these are `Population` methods (population/base.py), so
  a Deck has them. `map::the_population` (a die, or a deck's population as a die) is now `self` for the
  value-returning ones in statistics.rs: quantities, probabilities, quantity, probability, nearest, die_stat,
  standardized_moment, entropy, die_quantile. Methods that return a population (pad_to_denominator, group_by,
  marginals, ...) keep `the_die`: on a deck Python returns a Deck, not yet modelled there. The generators still never
  give these ops a deck (only the minimiser does); fixtures: t1e/population-stats-of-decks.

- **Counts beyond 64 bits (K6d).** Every multiset count is an `icepool_core::Int` (`int.rs`): an `i64` until
  checked arithmetic overflows, then an `Arc<IBig>`; canonical (fits `i64` => `Small`), so derived `Eq`/`Hash` are
  right. Not `Copy`: clone (cheap) or borrow; `+ - *` work on any mix of `Int`/`&Int`/`i64`, `floor_divmod` is
  Python's `divmod`, `Int == i64` / `Int < i64` compare directly. It runs through keep tuples (`Arc<[Int]>`),
  `pool::Count::{Int(Int), Tuple(Arc<[Int]>)}`, `Popped` counts, sizes, operator constants (`MultiplyCounts(Int)`
  ...) and statelets, the evaluator trait (`sizes: &[Option<Int>]`, `counts: &[Int]`) and every evaluator,
  `KeepIndex::Seq(Vec<Option<Int>>)`, `MultisetKeepIndex::Seq`, `KeepEvaluator.index: Option<Int>`, the program
  layer (`exact_int`, `int_arg` -> `Int`) and the facade (`convert.rs count_to_py / counts_to_py /
  count_from_py`; no `IntoPyObject` for `Int`, orphan rule). Indices and slice bounds stay `i64` (they saturate:
  slicing clamps, an index past 64 bits is out of range anyway); `Int::clamp_usize` is for `xs[:n]`-style slicing.
  Timings against the i64 build (icepool-run, best of 5): all 9.5k fixture programs 3.6s either way; heavy pool
  evaluations (d20 x 80 keep 40, largest_count of 80 d12, ...) 0-10% slower: `Int` is 16 bytes and its clone a
  branch, not a memcpy. `Hash` hashes a small value as its bare `i64`. If the long gate's timings suffer, the
  next lever is a keep tuple of plain `i64`s with an `Int` fallback per tuple, not per element.
- Where Python *builds* something `n` long from a count (`[0] * keep_size` in `make_keep_tuple`, `(x,) * n` in
  expand / argsort_grouped / all_straights / MultisetKeep, `(1,) * sum(keep_size)` for a compound keep, a tuple
  count `t * n`) use `keep_tuple::list_len` / `Int::to_len`: `OverflowError` for any `n` beyond 64 bits *whatever
  its sign* (CPython converts first: `(x,) * -2**70` raises; fuzz found it), 0 for other negatives, built up to
  2**24, `OverflowError` from 2**36 (Python: MemoryError, instantly), `Unsupported` in between (Python would
  allocate for seconds). `keep_size_as_len` checks a sequence index's length first (IndexError without building).
  `middle_slice` takes `&Int`s and saturates its bounds.
- Generators: `grammar/pool.py huge_count(g, negative)` picks from `HUGE_COUNTS` (2**62, 2**63-1, 2**63,
  2**64+1, 2**70): sums of these and small counts are small or past 2**61, never in the Unsupported band. Used by
  `_count` (pool_keep / die_keep sequences), die_pool sequences, pool_mul / pool_middle keep, pool_index /
  die_keep indices, and multiset.py's mx_counts constants, MultisetKeep counts and pool_mul. About 7% of the
  pool/multiset programs hold one. A filter-to-huge fuzz (generate, keep programs with |int| >= 2**61, compare)
  ran ~20k such programs clean natively and through run-facade.sh; fixtures/k6d has 95 hand-picked cases.

- **Long gate fix (L202610160934).** (1) wasm32: `usize` is 32 bits there, so `1 << 36` as a `usize` const broke
  the build and `usize::try_from(i64)` turned big lengths into 0. `Int::to_len` now returns a `u64` (Python's
  index-sized integer is 64 bits on every target) and `MEMORY_ERROR_SIZE` is a `u64`; callers bound a length
  before casting to `usize`. Write new size limits as `u64`/`i64`, never `usize` constants above 2**32, and run
  `cargo build -p icepool-wasm --target wasm32-unknown-unknown` when touching them (only the long gate does).
  (2) `consecutive()` over a range longer than `sys.maxsize` raises OverflowError (`tuple()` calls `len()` of
  the range); fixtures l202610160934/consecutive-overflow. (3) The minimiser can shrink a failure to a bare pool
  (its repr has an address, so it "fails" as unimplemented): don't `import-failures` that, take the `original`
  from harness-out/failures.jsonl and hand-minimise. (4) `grammar/pool.py` `_die_without_nan` also rejects dice
  whose outcomes sort differently from different starting orders (`_order_free`: Vectors of mixed element
  types): a pool sorts a set in per-process hash order, so the oracle itself raised TypeError on some runs only.
  `_has_nan` now looks inside Vectors too. (5) The long gate's "khepri sweep through wasm" failure was rustup
  (`rustc ... not applicable to the stable toolchain`, a toolchain update mid-run), not the port.

- **Recursion limits (R1).** `value/recursion.rs`: every recursion user data drives takes a `level()` (RecursionError
  past `MAX_DEPTH` = 10000 levels, CPython's C recursion limit, or `STACK_LIMIT` = 6 MiB of stack since the
  outermost level; WASM 1000 / 512 KiB), and icepool's *Python-level* recursion is emulated with `frames(n)`:
  the frames Python would push, checked against the host's budget (the facade registers `convert::frame_budget`
  = `sys.getrecursionlimit()` minus its stack depth, asked only past 16 emulated frames; native: 1000 with an empty
  stack). New `Error::Recursion` (was `Error::Value("RecursionError: ...")`).
  - Vector methods (`vector.rs`): the dunder + `binary_operator`/`unary_operator`/`reverse_binary_operator` (2,
    counted in `Vector::binary`/`reverse_binary`/`unary`/round, i.e. only when the core dispatches; the facade's
    calls come from those real Python frames), `Vector.__init__` + the generator (2, around elementwise results,
    not around the truth comparison). Calibrated: `t + t` raises from 250 levels at top level at any stack depth
    and recursion limit, also through a host callback that does vector ops itself (crates/icepool-py/tests).
  - Again (`again.rs`): `_evaluate` + genexpr + `_evaluate_arg` (3 a level), `DIE_CALL_FRAMES` (6) for the
    function in depth mode's outer levels (it gets a die), `replace_again` (+1) and, after the innermost level,
    `Die.__new__` + `evaluate_agains_using_depth` (+2); count mode: `make_step_outcome` (+1), 0 call frames.
    Matches the oracle for again_depth 0/1/2, again_end die/Reroll, again_count, at stack depths 0/7/100.
  - Keep recursive frames small: CPython's C recursion is lean, ours is not; `from_py`/`to_py`/`py_repr`/
    `PartialEq` dispatch tuples and vectors first and push everything else into `#[inline(never)]` helpers, and
    avoid `iter().map().collect::<Result<_>>()` on the recursive path (GenericShunt frames). Measure with
    crates/icepool-core/tests/recursion.rs (8 MiB thread, MAX_DEPTH+1 must raise, 2000 must work). An
    `eprintln!` of `&local as *const _ as usize` in `level()` at two depths gives bytes per level.
  - Infallible recursions (`PartialEq`, `Hash`, `Display`, `Drop`) take no level: every die outcome passed
    `check_key` (which does), `Display` prints `...` past the limit. The JS facade runs `program::check_nesting`
    on every Again / expression / raw-result handle it keeps, so chains built call by call stay bounded.
  - Facade cases: crates/icepool-py/tests/test_backends.py (`nest_vector`, `nest_tuple`, `nest_again`, `Deep`).
    Thresholds near 10000 nested tuples differ from CPython by a few levels (DIVERGENCES.md).
  - Not guarded (bounded elsewhere): MExpr's bool walkers (`has_parameter`, `same_expression`, `hashable`,
    `static_keepable`, Debug) — JSON depth natively, `check_nesting` in JS, unused by the Python facade;
    `map_tools` `has_nan`/`same_spelling`, facade `has_host` — values come through `from_py` (levels).
