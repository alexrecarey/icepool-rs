# icepool-wasm

Exact dice probabilities in JS: [icepool](https://github.com/HighDiceRoller/icepool) (Python) ported to Rust and
compiled to WASM, with a method-based JS API. Results are identical to Python icepool's (the same outcomes in the same
order, the same exact quantities); the test suite runs every golden fixture of the port through this API.

```js
import { d, Die, Pool, Deck, map, Again, Reroll } from "icepool-wasm";

const d6 = d(6);
d6.add(d6).items();                        // [[2, 1n], [3, 2n], ..., [12, 1n]]  (quantities are BigInts)
d6.mean();                                 // Fraction { numerator: 7n, denominator: 2n }
new Pool([d6, d6, d6, d6]).highest(3).sum().probability(">=", 15);
d6.reroll([1], { depth: 1 });              // keyword arguments are a trailing options object
map((a, b) => Math.max(a, b), d6, d6);     // JS callbacks
new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 2 });
new Deck([1, 2, 3], 4).deal(2).sum();
```

## Shape of the API

- **Arithmetic is methods**: `add sub mul truediv (div) floordiv mod pow lshift rshift and or xor`, comparisons
  `lt le gt ge eq ne` (dice of bools; `eq`/`ne` also have Python's truth value: `.truth()`), `neg pos abs invert
  round trunc floor ceil`. `Die.sum(n, die)` is Python's `n @ die`; `ops.add(a, b)` etc. apply Python's operators to
  any operands.
- **Python names in camelCase** (`probabilityWhere`, `keepCounts`, `largestStraight`...); Python's keyword arguments
  are a trailing options object (`{ star, depth, againDepth, percent, ... }`). Python's `'inf'` is `Infinity`.
- **Values**: ints are numbers (BigInts beyond 2**53), floats are numbers (an integral float such as `2.0` comes out
  as a `Float`, so that it stays a float), tuples are arrays, `None` is `null`; `Fraction`, `Vector`, `Symbols`,
  `Reroll`, `Restart`, `Break`, `Ellipsis` and `Again` are exported. Quantities and denominators are always BigInts.
- **Callbacks**: `map`, `reroll`, `filter`, `split`, `explode`, `groupBy`, `quantityWhere`, `keepOutcomes`,
  `MultisetExpression.mapCounts`... take JS functions. Like Python, `map` unpacks tuple outcomes (`star`) when the
  function needs more parameters than it is given; it counts `f.length`, so give an options parameter a default:
  `map((x, { k } = {}) => x + k, d6, { k: 1 })`.
- **Mappings**: where Python takes a dict (`map`, `reroll`, `groupBy`, `Wallenius`, `new Die(...)`...), pass a JS
  `Map`. Its keys are compared as Python's dict compares them: `1`, `1.0` and `true` are one key, and so are two
  equal arrays (tuples), which a JS `Map` would keep apart.
- **Random variables**: `fromRv(rv, outcomes, denominator, kwargs)` duck-types the rv as Python does
  (`scipy.stats`-like): `rv.cdf(xs, kwargs)` returns the cumulative probabilities; an rv with a `pdf` is continuous.
- **Evaluators**: subclass `MultisetEvaluator` (or call `evaluator({...})`) with `nextState(state, order, outcome,
  ...counts)` and optionally `initialState`, `finalOutcome`, `extraOutcomes`. States can be values, arrays or plain
  objects (keyed by their JSON). Throw `UnsupportedOrder` to ask for the other order.
- **Multiset functions** (`@multiset_function`): `multisetFunction((a, b) => [a.sub(b).size(), b.sub(a).size()])`
  returns a `MultisetFunction`; `f.call(pool1, pool2)` traces the JS function with parameter expressions (a
  `MultisetTupleExpression`, with `hand(i)`, for an unstarred `MultiDeal` input) and evaluates what it returns: an
  evaluation, an array of them (a joint evaluation), constants. Evaluations inside the function (expression methods,
  JS evaluators, other multiset functions) only record what to evaluate. Options: `params` (default `f.length`),
  `rest` (`(a, ...more) => ...`), `kwonly` (names of keyword-only arguments, passed to `f` as a trailing options
  object: `f.call(pool, { target: [5, 6] })`); `star` is inferred unless given.
- **Errors**: icepool's exceptions are thrown as `IcepoolError` with `pythonType` (`"ValueError"`, ...); an exception
  thrown by your callback reaches you unchanged.
- **Memory**: dice, pools and decks live in WASM memory and are freed when their JS objects are garbage-collected.

The TypeScript declarations are in `icepool.d.ts`.

## Building and testing (from the repository)

```bash
crates/icepool-wasm/js/build.sh          # wasm-pack builds into js/pkg-node and js/pkg-web (wasm-opt -O3)
crates/icepool-wasm/js/build.sh --test   # quick build, then the fixture suite, the API tests and the type check
crates/icepool-wasm/js/build.sh --pack   # optimised build, then npm pack
```

In node, `import ... from "icepool-wasm"` loads synchronously; in browsers and bundlers the module instantiates the
WASM with top-level await.
