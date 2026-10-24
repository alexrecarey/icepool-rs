// khepri timings through the node WASM build: the typical / mid / heavy cases of docs/ANALYSIS.md, inputs in
// bench-cases.json, each run as the harness's `khepri` node (icepool-harness's f2f engine) through `run`. Each case
// runs in a fresh instance first (as a new worker would), then warm.
//   node crates/icepool-wasm/bench.mjs <node package dir>   (run-wasm.sh --bench builds it and runs this)
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import path from "node:path";

import { bindRaw } from "./recover.mjs";

const require = createRequire(import.meta.url);
const entry = path.join(process.argv[2], "icepool_wasm.js");
const here = path.dirname(new URL(import.meta.url).pathname);
const cases = JSON.parse(readFileSync(path.join(here, "bench-cases.json"), "utf8"));

let t = performance.now();
let { run } = bindRaw(require(entry));
console.log(`instantiate: ${(performance.now() - t).toFixed(1)} ms`);
for (const [name, input] of Object.entries(cases)) {
  delete require.cache[require.resolve(entry)];
  ({ run } = bindRaw(require(entry)));
  const program = JSON.stringify({ op: "khepri", input });
  t = performance.now();
  const r = JSON.parse(run(program));
  const first = performance.now() - t;
  if (!r.ok) throw new Error(`${name}: ${JSON.stringify(r)}`);
  const warm = [];
  for (let i = 0; i < 3; i++) {
    t = performance.now();
    run(program);
    warm.push(performance.now() - t);
  }
  console.log(`${name}: first ${first.toFixed(1)} ms, warm best ${Math.min(...warm).toFixed(1)} ms ` +
    `(${r.die.length} outcomes)`);
}
