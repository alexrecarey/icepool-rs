// The icepool-run protocol through the WASM build: one JSON program per stdin line in, one JSON result per stdout
// line out. run-wasm.sh builds the node package and passes its directory as argv[2].
import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import path from "node:path";

const require = createRequire(import.meta.url);
const entry = path.join(process.argv[2], "icepool_wasm.js");

// A Rust panic traps the WASM instance (no unwinding in WASM) and may leave its memory mid-update, so after one we
// load a fresh instance, the way icepool-run reports a caught panic and carries on.
function load() {
  delete require.cache[require.resolve(entry)];
  return require(entry);
}
let wasm = load();

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of rl) {
  if (line.trim() === "") continue;
  let result;
  try {
    result = wasm.run(line);
  } catch (e) {
    // Same answer as icepool-run's panic guard; the cause goes to stderr for whoever debugs it.
    process.stderr.write(`run-wasm: ${e && e.message}\n`);
    result = JSON.stringify({ ok: false, error: "unimplemented: panic in icepool-run" });
    wasm = load();
  }
  process.stdout.write(result + "\n");
}
