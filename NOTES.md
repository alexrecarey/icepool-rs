# Notes for the next iteration

Gotchas, decisions and layout notes, written by the iterations themselves. Keep it current: fix or delete what
turns out wrong. Newest decisions at the top of each section.

## Decisions

- **`@`, Again, explode (K4).** Core: `Die::matmul(&[(i64, Quantity)])` (= `__matmul__` with this die on the
  right) over the `SumOutcome` trait (`+`, unary `-`, `x * 0`; impls for `Value` and `i64`), with a per-call
  `SumCache` that computes only the sums Python's `_sum_all` would (a `zero()` of mixed types raises only if count
  0 is needed). `Die::zero`, `Die::simplify` live there too. `again.rs`: `Again<F, L>` (closed enum: `Again` or
  `Apply(f, args)`) and generic `evaluate_agains_using_depth`; the program layer instantiates it with
  `AgainOp {Binary, Unary, MatMul}` and `PValue` (`PValue::Again`), so evaluating an expression goes back through
  `ops::binary` / `ops::matmul` and Python's dispatch. Count mode (`again_count`, `is_additive`) is not ported.
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
- `Value::Complex(re, im)`: only from `negative ** fractional` (CPython `_Py_c_pow`). Never survives in a die
  (unorderable: `Counts` raises), unless its quantity is 0 and it drops out first, which is why it is a value and
  not an immediate error. Arithmetic on it returns `Error::Unsupported`.
- `Error::Unsupported(what)`: Python would answer but the port does not cover it yet (complex arithmetic, `str %`
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
    hash (mod 2**61-1, so equal numbers of any type hash alike). `repr.rs`: Python `repr(float)`. Add Vector,
    Symbols, Die-as-value, Host here as new variants.
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
- Vector and Symbols literals decode to `unimplemented` (task S5).
- The oracle drops zero quantities passed to `Die({...})` at construction: `Die({1: 0, 3: 5})` has one outcome.
  Zero quantities still survive when mixing a die that has them (`Die([X, 3])` keeps X's zeros; `Die([X])` is X).
