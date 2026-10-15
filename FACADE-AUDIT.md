# Facade audit (S5b)

Every module of `python/icepool` under `ICEPOOL_BACKEND=rust`, at the end of S5b, rechecked by S5 (no change in
status: the exports S5 gave grammar nodes, `tupleize` / `middle` / `map_function` / Die's `Counts` views, already
ran on the core or composed core calls). Status:

- **core**: its computational work runs in `icepool_rs` (Python only juggles arguments, builds objects or holds data);
- **via**: no switch of its own, but it only composes operations that are on the core, or it is declarations / base
  classes / enums whose behaviour lives elsewhere;
- **partly**: some real work still runs in Python (listed below with the task that covers it);
- **dead**: unused on the rust backend (the core has its own version).

| module | status | notes |
|---|---|---|
| `__init__.py` | via | imports / exports, `Reroll` / `Restart` constants |
| `backend.py` | core | the switch itself (its docstring points here) |
| `collection/__init__.py` | via | empty |
| `collection/counts.py` | core | holds the Python copy of core-made items (`_from_sorted_items`); sorting / `remove_min` only on Python paths |
| `collection/symbols.py` | core | operators, comparisons, `count_subset`, `size`, `_str` via icepool_rs |
| `collection/vector.py` | core | unary / binary / reflected operators via icepool_rs; its module-level cartesian helpers are dead copies |
| `collection/vector_with_truth_only.py` | via | a truth value only |
| `creation_args.py` | core | `expand_args_for_die_rust` / `_deck_rust`; `itemize` / `expand_arg` are argument handling |
| `evaluator/__init__.py` | via | imports |
| `evaluator/argsort.py` | partly | not native: `next_state` called back from the core driver (K6c) |
| `evaluator/basic.py` | core | Sum (no map), Size, ProductOfCounts, Empty, Expand native; `SumEvaluator(map)` called back (K6c) |
| `evaluator/comparison.py` | core | all seven native |
| `evaluator/evaluator_rust.py` | core | the bridge (`rust_flats`, `rust_call_tree`, `builtin_spec`) |
| `evaluator/keep.py` | core | native for an index within i64 |
| `evaluator/lexi.py` | partly | `LexiComparisonEvaluator` called back (K6c) |
| `evaluator/multiset_evaluator.py` | via | base class; user methods are host callbacks of the core driver |
| `evaluator/multiset_evaluator_base.py` | core | `evaluate` on the core (`weightless()` too since S4e); upstream's driver for unrecognised sources (S5c) |
| `evaluator/multiset_function.py` | core | tracing in Python by design; the call tree runs in `evaluate_tree` |
| `evaluator/poker.py` | core | native except `LargestCountWithWild`, `AllStraightsReduceCounts` (K6c) |
| `expand.py` | partly | `tupleize` / `vectorize` / `cartesian_product` of dice on the core; public `iter_cartesian_product` and other argument kinds are Python loops (S5c) |
| `expression/__init__.py` | via | empty |
| `expression/multiset_expression.py` | via | builds operator / evaluator objects |
| `expression/multiset_expression_base.py` | via | base classes; their Python driver code is the fallback only |
| `expression/multiset_mixture.py` | via | one core evaluation per inner |
| `expression/multiset_parameter.py` | via | becomes `('param', i, star)` core nodes |
| `expression/multiset_tuple_expression.py` | core | `tuple_item` specs |
| `function.py` | core | coin, one_hot, pointwise_max/min, min/max_outcome, consecutive, sorted_union, harmonize_denominators' weighted_lcm on the core (S5b); d / z / `__getattr__`, stochastic_round, from_rv compose core calls; `from_cumulative`'s subtraction loop is Python on purpose (S1b) |
| `generator/__init__.py` | via | empty |
| `generator/compound_keep.py` | core | |
| `generator/deal.py` | core | |
| `generator/keep.py` | core | `keep()` via `_rs.keep` |
| `generator/multi_deal.py` | core | sources on the core; evaluators over it are host callbacks (tuple counts; S4f, S5c) |
| `generator/multiset_generator.py` | via | abstract base |
| `generator/multiset_tuple_generator.py` | via | abstract base |
| `generator/pool.py` | core | |
| `generator/weightless.py` | core | `WeightlessSource` holds an `icepool_rs.WeightlessSource` as `_rs` (pops, sizes, order on the core); the generator only wraps (S4e). A wrapped source without `_rs` (a user generator) stays Python: item 5 |
| `lexi.py` | via | computes the parameters of versus / pair specs |
| `map_tools/__init__.py` | via | empty |
| `map_tools/common.py` | via | `Break`, `TransitionType`, `transition_and_star` |
| `map_tools/core_impl.py` | core | `map_simple`; `TransitionCache` hands its args to the core |
| `map_tools/function.py` | partly | map (all modes), map_and_time, mean_time_to_absorb on the core; reduce / accumulate / map_function loop over the core `map`; `map_to_pool`'s loop and callbacks on the core, `Pool(...)` / `MultisetMixture` stay Python (S3c) |
| `map_tools/markov_chain.py` | core | `absorbing_markov_chain` |
| `math.py` | dead | only Python-backend paths |
| `operator/__init__.py` | via | imports |
| `operator/adjust_counts.py` | core | specs (map_counts functions are callbacks) |
| `operator/binary.py` | core | |
| `operator/debug.py` | core | |
| `operator/filter_outcomes.py` | core | |
| `operator/keep.py` | core | |
| `operator/multiset_operator.py` | via | base class |
| `operator/pair.py` | core | |
| `operator/versus.py` | core | |
| `order.py` | via | enums / exceptions; the order helpers only serve the Python driver |
| `population/__init__.py` | via | empty |
| `population/again.py` | core | both modes; expression functions are callbacks |
| `population/base.py` | partly | statistics, quantities, format, sample, group_by on the core; `append` / `remove` / `to_one_hot` and the Deck paths of `zero` / `split` are Python (S5c) |
| `population/deck.py` | core | `map` is a Python per-outcome loop before `Deck(...)` (S5c) |
| `population/die.py` | core | `sign` / `cmp` native (S2); `is_in` / `count` / `if_else` compose `map` as upstream does; `explode_to_pool` / `reroll_to_pool` compute their count / composition dice on the core and end in the core `map_to_pool` (S3c; `make_pool` is a host callback); `mean_time_to_sum` is a Python recursion over core calls (S5c) |
| `population/die_with_truth.py` | via | lazy calls into the core |
| `population/format.py` | core | csv writing by Python's csv module by design |
| `population/keep.py` | via | `_sum_slice` dispatches to core `+`, `_lowest/_highest_single`, pools |
| `typing.py` | via | protocols, enums, `infer_star` |
| `wallenius.py` | partly | Deck / Die / Deal / mixtures on the core; `Deck.append` / `Die.remove` are Counter arithmetic (S5c) and `deal` ends in the core `map_to_pool` (S3c) |

## Real work still in Python under rust

1. ~~`map_to_pool`~~: on the core since S3c (the result's `Pool(...)` and `MultisetMixture` are facade classes).
2. ~~Evaluations over `weightless()` generators~~: on the core since S4e.
3. Evaluators called back from the core driver: `SumEvaluator(map)`, `LargestCountWithWild`,
   `AllStraightsReduceCounts`, argsort, `LexiComparisonEvaluator`, keep with an index beyond i64: **K6c**.
4. Evaluators over a `MultiDeal` (tuple counts): operators **S4f**, built-ins **S5c**.
5. Sources the core does not recognise (user generator subclasses, operator subclasses without a spec) use
   upstream's driver silently: **S5c**.
6. `iter_cartesian_product` and `cartesian_product` over decks / expressions / other types: **S5c**.
7. `Population.append` / `remove`, `to_one_hot`, the Deck paths of `zero` / `split`, `Deck.map`: **S5c**.
8. `Die.mean_time_to_sum`: **S5c**.
9. By design, not tasks: `from_cumulative`'s subtraction loop (S1b), `_select_outcomes` (predicates and `in` stay
   Python's, the core gets flags), `@multiset_function` tracing, `infer_star`.
