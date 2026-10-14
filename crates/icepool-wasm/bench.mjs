// khepri timings through the node WASM build: the typical / mid / heavy cases of docs/ANALYSIS.md, inputs in
// bench-cases.json. Each case runs in a fresh instance first (as a new worker would), then warm.
//   node crates/icepool-wasm/bench.mjs <node package dir>   (run-wasm.sh --bench builds it and runs this)
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import path from "node:path";

const require = createRequire(import.meta.url);
const entry = path.join(process.argv[2], "icepool_wasm.js");
const here = path.dirname(new URL(import.meta.url).pathname);
const cases = JSON.parse(readFileSync(path.join(here, "bench-cases.json"), "utf8"));

let t = performance.now();
let wasm = require(entry);
console.log(`instantiate: ${(performance.now() - t).toFixed(1)} ms`);
for (const [name, input] of Object.entries(cases)) {
  delete require.cache[require.resolve(entry)];
  wasm = require(entry);
  t = performance.now();
  const r = wasm.calculate(input);
  const first = performance.now() - t;
  const warm = [];
  for (let i = 0; i < 3; i++) {
    t = performance.now();
    wasm.calculate(input);
    warm.push(performance.now() - t);
  }
  console.log(`${name}: first ${first.toFixed(1)} ms, warm best ${Math.min(...warm).toFixed(1)} ms ` +
    `(rolls ${r.rolls}, ${r.outcomes.length} outcomes)`);
}
