---
title: Guide
---

# The icepool-wasm guide

icepool-wasm computes exact dice probabilities in JavaScript and TypeScript. It is the Python library
[icepool](https://github.com/HighDiceRoller/icepool) ported to Rust and compiled to WebAssembly, with an API that
mirrors Python icepool's: the same classes and functions, in JS's conventions. Results are Python icepool's exactly:
the same outcomes in the same order and the same integer quantities (the port's test suite runs every one of its
golden fixtures, computed by Python icepool, through this API).

This guide walks through the library. The API reference (`npm run docs`, see the README) documents every symbol, and
Python icepool's [documentation](https://highdiceroller.github.io/icepool/) explains the concepts in more depth:
what it says applies here, with the translations of [Differences from Python
icepool](#differences-from-python-icepool).

Every example on this page is run by the package's tests. A comment `//=> x` shows what the line before it
evaluates to.

## Install

```sh
npm install icepool-wasm
```

```ts
import { d, Die, Pool } from "icepool-wasm";
```

In node (18 or later) the import loads the WebAssembly module synchronously. In browsers and bundlers the module
instantiates it with top-level await, so import it from an ES module; bundlers that understand
`new URL("...wasm", import.meta.url)` (Vite, webpack 5, esbuild with a loader for `.wasm` files) find the file by
themselves. The package is ESM only and ships its TypeScript declarations.

## First dice

`d(6)` is a six-sided die. Dice combine with methods named after Python's operators (`add`, `sub`, `mul`, `gt`,
...), since JavaScript has no operator overloading:

```ts
import { d, Die } from "icepool-wasm";

const d6 = d(6);
const twoD6 = d6.add(d6);
twoD6.outcomes(); //=> [2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
twoD6.quantity(7); //=> 6n
twoD6.denominator(); //=> 36n
twoD6.probability(7).toString(); //=> "1/6"
twoD6.probability(">=", 10).toString(); //=> "1/6"
twoD6.mean().toString(); //=> "7"
```

A die is a finite distribution: a sorted list of outcomes, each with an integer quantity (its weight). Probabilities
are exact `Fraction`s (`Number(fraction)` gives a float), as in Python; `{ percent: true }` gives a percentage as a
number.

`Die.sum(n, die)` (Python's `n @ die`) rolls a die `n` times and sums. When the left side of a Python operator is a
plain number, use the reflected method: `3 - d6` is `d6.rsub(3)`, `3 @ d6` is `d6.rmatmul(3)`.

```ts
const threeD6 = Die.sum(3, d(6));
threeD6.equals(d(6).rmatmul(3)); //=> true
threeD6.probability("<=", 5).toString(); //=> "5/108"
d(6).rsub(7).equals(d(6)); //=> true
```

Comparisons are dice of booleans:

```ts
d(20).ge(15).items(); //=> [[false, 14n], [true, 6n]]
```

`eq` and `ne` are dice too, which also carry Python's truth value (`d6 == d6` is a die *and* `True` in Python):
`.truth()` reads it. `equals` compares two dice as values.

```ts
d(6).eq(d(6)).items(); //=> [[false, 30n], [true, 6n]]
d(6).eq(d(6)).truth(); //=> true
d(6).equals(d(6)); //=> true
```

Dice can be built from a list of outcomes (repeats add up), a list with `times`, or a `Map` of outcome to quantity:

```ts
new Die([1, 2, 2, 3]).items(); //=> [[1, 1n], [2, 2n], [3, 1n]]
new Die([1, 2, 3], [1, 2, 1]).items(); //=> [[1, 1n], [2, 2n], [3, 1n]]
new Die(new Map([[1, 1n], [2, 2n], [3, 1n]])).items(); //=> [[1, 1n], [2, 2n], [3, 1n]]
```

Outcomes can be numbers, strings, booleans, `null`, Fractions, arrays (Python's tuples), `Vector`s and `Symbols`;
arithmetic on arrays is not elementwise (use `Vector` for that). Formatting follows Python's `str` and `format`:

```ts
d(4).toString().split("\n")[0]; //=> "Die with denominator 4"
d(4).repr(); //=> "Die({1: 1, 2: 1, 3: 1, 4: 1})"
d(2).format("csv"); //=> "Outcome,Quantity,Probability\r\n1,1,50.000000%\r\n2,1,50.000000%\r\n"
```

### Statistics

```ts
const d6 = d(6);
d6.mean().toString(); //=> "7/2"
d6.variance().toString(); //=> "35/12"
d6.sd(); //=> 1.707825127659933
d6.median(); //=> 3.5
d6.mode(); //=> [1, 2, 3, 4, 5, 6]
d6.quantities("<="); //=> [1n, 2n, 3n, 4n, 5n, 6n]
```

### Mapping outcomes

`map` applies a function to every outcome; with several dice the function takes one outcome of each, over every
combination. A function may also return a die (the result is a mixture), `Reroll`, or a tuple (an array).

```ts
import { d, map, Reroll } from "icepool-wasm";

d(6).map((x) => x * 2).outcomes(); //=> [2, 4, 6, 8, 10, 12]
map((a, b) => Math.max(a, b), d(6), d(6)).probability(6).toString(); //=> "11/36"
d(6).map((x) => (x === 1 ? Reroll : x)).outcomes(); //=> [2, 3, 4, 5, 6]
```

When the outcomes are tuples and the function takes several parameters, `map` unpacks them, as Python's `star`
inference does (it counts `f.length`, so an options parameter needs a default):

```ts
const pairs = map((a, b) => [a, b], d(2), d(2));
pairs.map((a, b) => a + b).items(); //=> [[2, 1n], [3, 2n], [4, 1n]]
```

### Rerolls and filters

Python's keyword arguments are a trailing options object; `depth` is required (an int, or `Infinity` for Python's
`'inf'`):

```ts
d(6).reroll([1], { depth: Infinity }).outcomes(); //=> [2, 3, 4, 5, 6]
d(6).reroll((x) => x <= 2, { depth: 1 }).items(); //=> [[1, 2n], [2, 2n], [3, 8n], [4, 8n], [5, 8n], [6, 8n]]
d(20).filter((x) => x > 10, { depth: Infinity }).outcomes().length; //=> 10
```

## Pools and evaluators

A pool is several dice rolled together, without summing them. Keep the highest or lowest, then evaluate the kept
dice: a sum, a count, a straight, a set...

```ts
import { d, Pool } from "icepool-wasm";

// 4d6, keep the highest 3.
const stats = d(6).pool(4).highest(3).sum();
stats.mean().toString(); //=> "15869/1296"
stats.probability(">=", 15).toString(); //=> "25/108"
stats.equals(new Pool([d(6), d(6), d(6), d(6)]).highest(3).sum()); //=> true
```

Python's `pool[-2:]` (the highest two) is `pool.keep(-2, null)`, and `pool[i]` is `pool.keep(i)`. A pool may mix
dice: `new Pool([d(6), d(8)])`.

The built-in evaluations are methods of pools and of the multiset expressions made from them (`sum`, `size`,
`largestCount`, `largestStraight`, `allCounts`, `expand`, ...); each returns a die:

```ts
// Matching sets in 5d6: the size of the largest group of equal dice.
d(6).pool(5).largestCount().items(); //=> [[1, 720n], [2, 5400n], [3, 1500n], [4, 150n], [5, 6n]]
// How many of 4d6 roll at least 5.
d(6).pool(4).keepOutcomes([5, 6]).size().mean().toString(); //=> "4/3"
```

Multiset expressions combine pools as multisets (Python's `|`, `&`, `-`, `+` on pools are `or`, `and`, `sub`,
`add`), and the result is evaluated the same way:

```ts
// Dice of the first pool that the second pool does not match.
const unmatched = d(6).pool(3).sub(d(6).pool(2)).size();
unmatched.mean().toString(); //=> "319/144"
```

## Decks

A `Deck` is a population drawn without replacement: `new Deck(outcomes, times)`. `deal(n)` deals a hand, which is
evaluated like a pool:

```ts
import { Deck } from "icepool-wasm";

// Four copies each of 1..10; deal 5 and sum.
const deck = new Deck([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 4);
deck.deal(5).sum().mean().toString(); //=> "55/2"
// The chance that a hand of 2 from [1, 1, 2, 2] is a pair.
new Deck([1, 2], 2).deal(2).largestCount().probability(2).toString(); //=> "1/3"
```

`MultiDeal` deals several hands from one deck at once; `multisetFunction` (below) evaluates them together.

## Again and explode

`Again` in a die's outcomes means "roll this die again and use that". It is an expression object with the same
methods as a die: `Again.add(6)` is Python's `Again + 6`. `againDepth` bounds the recursion (Python's
`again_depth`, 1 by default).

```ts
import { d, Die, Again } from "icepool-wasm";

// A d6 that explodes on a 6, up to twice.
const exploding = new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 2 });
exploding.outcomes(); //=> [1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 13, 14, 15, 16, 17, 18]
exploding.equals(d(6).explode([6], { depth: 2 })); //=> true
```

`explode` is the shortcut: `d(6).explode()` explodes the highest outcome, `depth` 9 by default.

```ts
d(6).explode().mean().toString(); //=> "84652645/20155392"
```

## Custom evaluators in JavaScript

When no built-in evaluation does what you need, write an evaluator. As in Python, an evaluator sees the outcomes of
the pool one at a time, in order, with how many dice rolled each, and folds them into a state; icepool turns that
into a die of final outcomes efficiently, without enumerating the rolls.

Subclass `MultisetEvaluator` and implement `nextState(state, order, outcome, ...counts)`; optionally
`initialState(order, outcomes, ...sizes)` and `finalOutcome(state, order, outcomes, ...sizes)`:

```ts
import { d, MultisetEvaluator, type OrderValue } from "icepool-wasm";

/** How many dice rolled a 6. */
class CountSixes extends MultisetEvaluator {
  override initialState(): number {
    return 0;
  }
  override nextState(state: number, order: OrderValue, outcome: number, count: number): number {
    return outcome === 6 ? state + count : state;
  }
}

new CountSixes().evaluate(d(6).pool(3)).items(); //=> [[0, 125n], [1, 75n], [2, 15n], [3, 1n]]
```

`evaluator({ ... })` makes one from plain functions:

```ts
import { d, evaluator } from "icepool-wasm";

const sum = evaluator({
  initialState: () => 0,
  nextState: (state: number, order: unknown, outcome: number, count: number) => state + outcome * count,
});
sum.evaluate(d(6).pool(2)).equals(d(6).add(d(6))); //=> true
```

States must be hashable the way Python's are: numbers, strings, booleans, `null`, Fractions, and (nested) arrays of
them; plain objects work too (they are compared by their JSON). Throw `UnsupportedOrder` from `nextState` to ask
for the other order, as a Python evaluator raises it.

`multisetFunction` is Python's `@multiset_function`: the function receives parameter expressions, combines them
with expression methods, and returns an evaluation (or an array of them, evaluated jointly); `call` evaluates it on
pools:

```ts
import { d, multisetFunction, type MultisetExpression } from "icepool-wasm";

const unmatched = multisetFunction((a: MultisetExpression, b: MultisetExpression) => a.sub(b).size());
unmatched.call(d(6).pool(3), d(6).pool(2)).mean().toString(); //=> "319/144"
```

In TypeScript, annotate the parameters: `MultisetExpression`, or `MultisetTupleExpression` for an unstarred
`MultiDeal` argument (whose `hand(i)` is one hand). The number of parameters is `f.length` unless `params` says
otherwise; `rest: true` takes `(a, ...more) =>`, and `kwonly` names keyword-only arguments, which reach `f` as a
trailing options object (`f.call(pool, { target: 5 })`).

## Values and BigInt quantities

Quantities and denominators are always `bigint`, because they outgrow `Number.MAX_SAFE_INTEGER` quickly:

```ts
import { d, Die, Float, Fraction } from "icepool-wasm";

Die.sum(30, d(6)).denominator(); //=> 221073919720733357899776n
```

Outcomes keep Python's types:

| Python | JS |
|---|---|
| `int` | `number`; `bigint` beyond 2\*\*53 (either is accepted going in) |
| `float` | `number`; an integral float comes out as a `Float`, so that `2.0` stays a float |
| `Fraction`, `complex` | `Fraction` (bigint numerator and denominator), `Complex` |
| `tuple` | array |
| `Vector`, `Symbols` | `Vector`, `Symbols` |
| `None`, `bool`, `str` | `null`, `boolean`, `string` |
| `dict` | `Map` (keys compared as Python's dict does: `1`, `1.0` and `true` are one key) |
| `'inf'` (a depth) | `Infinity` |

```ts
d(4).truediv(2).outcomes()[0]; //=> 0.5
d(4).truediv(2).outcomes()[1] instanceof Float; //=> true
new Die([2n ** 60n]).outcomes(); //=> [1152921504606846976n]
String(new Die([new Fraction(6n, 4n)]).outcomes()[0]); //=> "3/2"
```

## Disposing

Dice, pools, decks and expressions live in WebAssembly memory. Each is freed when its JS object is
garbage-collected, or at once by `free()`. They are also `Disposable`, so in TypeScript 5.2+ (and runtimes with
explicit resource management) `using die = d(6);` frees the die at the end of the block. Using an object after
freeing it throws a `TypeError`; objects made from it are not affected.

```ts
import { d } from "icepool-wasm";

const big = d(1000);
const total = big.add(big);
big.free();
total.denominator(); //=> 1000000n
big.mean(); //=> throws TypeError
```

Free explicitly in long loops: the garbage collector frees WASM memory only when it gets round to the JS objects,
and WASM memory grows meanwhile. Always free an object whose JS callback refers back to it (such as a multiset
function that calls itself): the core keeps such a callback until the object is freed, so the collector never
frees either.

## Errors

icepool's exceptions are thrown as `IcepoolError`, whose `name` and `pythonType` are the Python exception Python
icepool raises and whose `kind` says what went wrong:

```ts
import { Die, IcepoolError } from "icepool-wasm";

try {
  new Die([]).mean();
} catch (e) {
  (e instanceof IcepoolError); //=> true
  (e as IcepoolError).pythonType; //=> "ZeroDivisionError"
}
```

Subclasses `OrderError`, `ConflictingOrderError` and `UnsupportedOrder` match Python's. An exception thrown by your
own callback reaches you unchanged. Sizes no memory holds throw a `MemoryError`:

```ts
d(2n ** 60n); //=> throws MemoryError
```

A bug in the library itself (a Rust panic, which stops the WebAssembly instance) is thrown as an `IcepoolError`
with `kind` `"Panic"` and `pythonType` `"PanicException"` (what the Python facade raises for one); the instance
and all your objects stay usable afterwards.

## Differences from Python icepool

The API is Python icepool's, translated by these rules:

- **Names are camelCase** (`keep_outcomes` is `keepOutcomes`); class names are Python's.
- **Operators are methods** named after Python's dunder methods: `add sub mul truediv floordiv mod pow matmul neg
  pos abs invert and or xor lshift rshift lt le gt ge eq ne`, and the reflected `radd rsub rmul ...` for a plain
  value on the left. `ops.add(a, b)` etc. apply Python's operators to any operands, plain values included.
- **Keyword arguments are a trailing options object**: `die.explode([6], { depth: 2 })`. Positional arguments stay
  positional, `*args` are rest parameters.
- **Indexing and slicing** are methods: `pool[-2:]` is `pool.keep(-2, null)`, `pool[0]` is `pool.keep(0)`;
  `x in die` is `die.has(x)`, `len(die)` is `die.size`, `die[x]` is `die.get(x)`.
- **`==` between dice** is `eq` (a die, with `.truth()` for Python's truth value); `equals` compares dice as values.
- **Values** follow the table above; `None` is `null`, `'inf'` is `Infinity`, dicts are `Map`s.
- **Callbacks**: `star` is inferred from the function's `length` (its required parameters), as Python counts
  positional parameters. Python's keyword arguments to a callback arrive as a trailing options object.
- **Evaluators** subclass `MultisetEvaluator` (or use `evaluator({...})`); `@multiset_function` is
  `multisetFunction(f)` and its result is called with `.call(...)`.
- **Errors** are `IcepoolError`s carrying the Python exception's name; JS's own `TypeError` is thrown for using a
  freed object.
- **Memory** is freed with `free()` / `using`, or eventually by the garbage collector.
- **No caches across calls**: Python icepool caches some results (`d(6)` is the same object each time); here each
  call makes a new object, equal by `equals`.
- The library is synchronous and single-threaded; a long computation blocks its thread (run it in a worker for a
  responsive page).
