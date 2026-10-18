# icepool-wasm

Exact dice probabilities in JavaScript and TypeScript: [icepool](https://github.com/HighDiceRoller/icepool)
(Python) ported to Rust and compiled to WebAssembly, with an API mirroring Python icepool's. Results are identical
to Python icepool's: the same outcomes in the same order, with the same exact quantities. The test suite runs every
golden fixture of the port, computed by Python icepool, through this API.

```sh
npm install icepool-wasm
```

The package is not on npm yet. Until it is, build it from the repository with `crates/icepool-wasm/js/build.sh
--pack` and install the tarball that writes (`npm install path/to/icepool-wasm-0.1.0.tgz`).

```ts
import { d, Die, Deck, Again, map } from "icepool-wasm";

const d6 = d(6);
d6.add(d6).probability(7).toString(); //=> "1/6"
d6.add(d6).items()[0]; //=> [2, 1n]
d6.pool(4).highest(3).sum().mean().toString(); //=> "15869/1296"
d6.reroll([1], { depth: 1 }).probability(1).toString(); //=> "1/36"
map((a, b) => Math.max(a, b), d6, d6).probability(6).toString(); //=> "11/36"
new Die([1, 2, 3, 4, 5, Again.add(6)], { againDepth: 2 }).equals(d6.explode([6], { depth: 2 })); //=> true
new Deck([1, 2, 3, 4], 2).deal(3).sum().mean().toString(); //=> "15/2"
```

## Documentation

- [The guide](docs/guide.md): install, first dice, pools and evaluators, decks, Again and explode, custom evaluators
  in JS, BigInt quantities, disposing, errors, and the differences from Python icepool.
- The API reference: every exported symbol, with an example. `npm run docs` builds it with TypeDoc into
  `docs/api/` (from the repository, after `npm install`).
- Python icepool's [documentation](https://highdiceroller.github.io/icepool/) for the concepts.

Every TypeScript example in this README, the guide and the API reference is run by the test suite.

## The API in brief

The classes and functions are Python icepool's, translated predictably:

- names in camelCase (`keepOutcomes`); operators as methods (`d6.add(1)`, `d6.gt(3)`, `d6.rsub(7)` for `7 - d6`);
- Python's keyword arguments as a trailing options object (`die.explode([6], { depth: 2 })`); `'inf'` is `Infinity`;
- quantities and denominators are `bigint`; ints are numbers (BigInts beyond 2\*\*53); tuples are arrays, dicts are
  `Map`s, `None` is `null`;
- callbacks are plain JS functions; evaluators subclass `MultisetEvaluator`;
- errors are `IcepoolError`s named after the Python exception;
- objects live in WASM memory: `free()` (or `using`) releases them at once, else the garbage collector does.

## Building and testing (from the repository)

```sh
crates/icepool-wasm/js/build.sh          # wasm-pack builds into js/pkg-node and js/pkg-web (wasm-opt -O3), then tsc
crates/icepool-wasm/js/build.sh --test   # quick build, then the fixture suite, the API tests (with upstream's tests
                                         # translated, test/upstream-*.test.mjs), the documentation's examples
                                         # (test/doc-examples.test.mjs) and the type checks
crates/icepool-wasm/js/build.sh --docs   # quick build, then the API reference into js/docs/api (npm run docs)
crates/icepool-wasm/js/build.sh --pack   # optimised build, then npm pack
```

Building needs wasm-pack and TypeScript (`npm install` in this directory, or a `tsc` on the PATH); the
documentation needs `npm install` (TypeDoc). The API is written in TypeScript (`src/*.ts`); tsc compiles it into
`dist/`, declarations included. In node, `import ... from "icepool-wasm"` loads synchronously; in browsers and
bundlers the module instantiates the WASM with top-level await.
