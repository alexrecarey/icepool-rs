// The icepool-run protocol through the WASM build: one JSON program per stdin line in, one JSON result per stdout
// line out. run-wasm.sh builds the node package and passes its directory as argv[2].
import { createRequire } from "node:module";
import { createInterface } from "node:readline";
import path from "node:path";
import { Panic, bindRaw } from "./recover.mjs";

// `run` recovers from a trap the way every call of the JS API does (the shadow stack pointer put back, the core's
// per-call state reset), so the instance stays usable.
const { run } = bindRaw(createRequire(import.meta.url)(path.join(process.argv[2], "icepool_wasm.js")));

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of rl) {
  if (line.trim() === "") continue;
  let result;
  try {
    result = run(line);
  } catch (e) {
    if (!(e instanceof Panic)) throw e;
    // Same answer as icepool-run's panic guard; the panic's message goes to stderr for whoever debugs it.
    process.stderr.write(`run-wasm: ${e.message}\n`);
    result = JSON.stringify({ ok: false, error: "unimplemented: panic in icepool-run" });
  }
  process.stdout.write(result + "\n");
}
