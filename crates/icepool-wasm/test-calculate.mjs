// calculate() through the node WASM build against what khepri-f2f's f2f.calculate returned on the oracle
// (crates/icepool-khepri/tests/khepri_calculate.json: 40 inputs): rolls exactly, every chance to the bit.
//   node crates/icepool-wasm/test-calculate.mjs <node package dir>   (run-wasm.sh --test builds it and runs this)
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";

import { bindRaw } from "./recover.mjs";

// With the recovery from a trap that the JS package's `calculate` (js/src/wasm.ts) has, which khepri's worker
// should call.
const { calculate } = bindRaw(createRequire(import.meta.url)(path.join(process.argv[2], "icepool_wasm.js")));
const here = path.dirname(new URL(import.meta.url).pathname);

// rolls passes 2**53, where JSON.parse would round: keep its source text (node >= 21 passes it to the reviver).
const cases = JSON.parse(
  readFileSync(path.join(here, "../icepool-khepri/tests/khepri_calculate.json"), "utf8"),
  (key, value, context) => (key === "rolls" ? BigInt(context.source) : value),
);
assert.equal(cases.length, 40);
let big = 0;
for (const { input, expected } of cases) {
  const got = calculate(input);
  // A Number while it is a safe integer, a BigInt beyond (as Pyodide's to_js gives a Python int).
  assert.equal(typeof got.rolls, Number.isSafeInteger(Number(expected.rolls)) ? "number" : "bigint");
  if (typeof got.rolls === "bigint") big++;
  assert.equal(BigInt(got.rolls), expected.rolls, JSON.stringify(input));
  assert.deepEqual(got.outcomes, expected.outcomes, JSON.stringify(input));
  for (const [g, e] of got.outcomes.map((o, i) => [o, expected.outcomes[i]])) {
    assert.ok(Object.is(g.chance, e.chance));
  }
}
// Where Python raises, calculate throws.
assert.throws(() => calculate({ ...cases[0].input, burstA: undefined }));
assert.throws(() => calculate("not an object"));
console.log(`calculate: ${cases.length} cases match f2f.calculate (${big} with rolls past 2**53)`);
