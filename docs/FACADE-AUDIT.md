# Facade audit (S5b, updated by S5c)

Every module of `python/icepool` under `ICEPOOL_BACKEND=rust`, at the end of S5b, rechecked by S5 (no change in
status: the exports S5 gave grammar nodes, `tupleize` / `middle` / `map_function` / Die's `Counts` views, already
ran on the core or composed core calls), then by S5c (the leftovers it moved to the core are marked). Status:

- **core**: its computational work runs in `icepool_rs` (Python only juggles arguments, builds objects or holds data);
- **via**: no switch of its own, but it only composes operations that are on the core, or it is declarations / base
  classes / enums whose behaviour lives elsewhere;
- **partly**: some real work still runs in Python (listed below with the task that covers it);
- **dead**: unused on the rust backend (the core has its own version).

| module | status | notes |
|---|---|---|
| `__init__.py` | via | imports / exports, `Reroll` / `Restart` constants |
| `backend.py` | core | the switch itself (its docstring points here; brought up to date by S5c) |
| `collection/__init__.py` | via | empty |
| `collection/counts.py` | core | holds the Python copy of core-made items (`_from_sorted_items`); sorting / `remove_min` only on Python paths |
| `collection/symbols.py` | core | operators, comparisons, `count_subset`, `size`, `_str` via icepool_rs |
| `collection/vector.py` | core | unary / binary / reflected operators via icepool_rs; upstream's dead module-level cartesian helpers are left out (S5c) |
| `collection/vector_with_truth_only.py` | via | a truth value only |
| `creation_args.py` | core | `expand_args_for_die_rust` / `_deck_rust`; `itemize` / `expand_arg` are argument handling |
| `evaluator/__init__.py` | via | imports |
| `evaluator/argsort.py` | core | native (`builtin_spec`: an `Order` and an i64 `limit`), over tuple counts too since S5e |
| `evaluator/basic.py` | core | Sum (no map), Size, ProductOfCounts, Empty, Expand native; `SumEvaluator(map)` called back (K6c) |
| `evaluator/comparison.py` | core | all seven native |
| `evaluator/evaluator_rust.py` | core | the bridge (`rust_flats`, `rust_call_tree`, `builtin_spec`) |
| `evaluator/keep.py` | core | native for an index within i64 |
| `evaluator/lexi.py` | core | `LexiComparisonEvaluator` native for an `Order` and an i64 lexi tuple, over tuple counts too since S5e |
| `evaluator/multiset_evaluator.py` | via | base class; user methods are host callbacks of the core driver |
| `evaluator/multiset_evaluator_base.py` | core | `evaluate` on the core: user sources and operators as host ones (S5c); dungeon / dungeonlet classes the core does not know raise `NotImplementedError` (no fallback to upstream's driver) |
| `evaluator/multiset_function.py` | core | tracing in Python by design; the call tree runs in `evaluate_tree` |
| `evaluator/poker.py` | core | native except `LargestCountWithWild`, `AllStraightsReduceCounts` (K6c) |
| `expand.py` | core | `iter_cartesian_product` and `cartesian_product` for every argument kind and outcome type on the core (S5c; `outcome_type` other than tuple is called back); `arg_items` / the population type are argument juggling |
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
| `generator/multi_deal.py` | core | sources on the core; built-in evaluators over it on the core since S5d (argsort / lexi since S5e) |
| `generator/multiset_generator.py` | via | abstract base; a user subclass's source is a host source of the core driver (S5c) |
| `generator/multiset_tuple_generator.py` | via | abstract base |
| `generator/pool.py` | core | |
| `generator/weightless.py` | core | `WeightlessSource` holds an `icepool_rs.WeightlessSource` as `_rs` (pops, sizes, order on the core); the generator only wraps (S4e). A wrapped user source is a host source inside it (S5c) |
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
| `operator/multiset_operator.py` | via | base class; a user subclass (or a subclass of a built-in operator) is a host operator of the core driver (S5c) |
| `operator/pair.py` | core | |
| `operator/versus.py` | core | |
| `order.py` | via | enums / exceptions; the order helpers only serve the Python driver |
| `population/__init__.py` | via | empty |
| `population/again.py` | core | both modes; expression functions are callbacks |
| `population/base.py` | core | statistics, quantities, format, sample, group_by on the core; `append` / `remove` / `to_one_hot` and the Deck paths of `zero` / `split` too since S5c (`to_one_hot`'s `len` / `in` / `index` of a given sequence are Python's, called back) |
| `population/deck.py` | core | `map`'s calls are made from the core (S5c); its star / mapping juggling and `Deck(...)` as upstream |
| `population/die.py` | core | `sign` / `cmp` native (S2); `is_in` / `count` / `if_else` compose `map` as upstream does; `explode_to_pool` / `reroll_to_pool` compute their count / composition dice on the core and end in the core `map_to_pool` (S3c; `make_pool` is a host callback); `mean_time_to_sum` on the core since S5c (extending the die's own cache list) |
| `population/die_with_truth.py` | via | lazy calls into the core |
| `population/format.py` | core | csv writing by Python's csv module by design |
| `population/keep.py` | via | `_sum_slice` dispatches to core `+`, `_lowest/_highest_single`, pools |
| `typing.py` | via | protocols, enums, `infer_star` |
| `wallenius.py` | via | Deck / Die / Deal / mixtures on the core; `Deck.append` / `Die.remove` on the core since S5c; `deal` ends in the core `map_to_pool` (S3c) |

## Real work still in Python under rust

1. ~~`map_to_pool`~~: on the core since S3c (the result's `Pool(...)` and `MultisetMixture` are facade classes).
2. ~~Evaluations over `weightless()` generators~~: on the core since S4e.
3. Evaluators called back from the core driver: `SumEvaluator(map)`, `LargestCountWithWild`,
   `AllStraightsReduceCounts`, argsort, `LexiComparisonEvaluator`, keep with an index beyond i64: **K6c**.
4. ~~Evaluators over a `MultiDeal` (tuple counts)~~: operators on the core since S4f, the built-in evaluators
   since S5d (natively in the facade and icepool-run, also fed tuple counts by a host operator), argsort and lexi
   since S5e.
5. ~~Sources the core does not recognise use upstream's driver silently~~: since S5c user sources are host sources
   and operators without a spec host operators of the core driver; an evaluation prepared into dungeon /
   dungeonlet classes the core does not know (a user's own `_prepare`, internal API) raises `NotImplementedError`.
   Facade gaps on these paths: counts / sizes a user source or operator yields that are not `int`s or tuples of
   `int`s within 64 bits raise `NotImplementedError`; a user operator gets no source / argument counts (an empty
   iterator and tuple: an operator only reads its children's).
6. ~~`iter_cartesian_product` and `cartesian_product` over decks / expressions / other types~~: on the core since S5c.
7. ~~`Population.append` / `remove`, `to_one_hot`, the Deck paths of `zero` / `split`, `Deck.map`~~: on the core
   since S5c.
8. ~~`Die.mean_time_to_sum`~~: on the core since S5c.
9. By design, not tasks: `from_cumulative`'s subtraction loop (S1b), `_select_outcomes` (predicates and `in` stay
   Python's, the core gets flags), `@multiset_function` tracing, `infer_star`.
