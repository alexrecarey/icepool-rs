// Test-only: the low level the JS tests reach (test/fixtures.mjs translates fixture programs into calls of the
// public API, and uses these for what has no public counterpart). Not part of the package's exports map.

export { Handle, RawJSON, call, fromWire, liveHandles } from "./rpc.js";
export { run } from "./wasm.js";

import { wasm } from "./wasm.js";

/** How many core objects JS holds: handles of the bridge and typed dice (for leak checks). */
export function liveObjects(): number {
  return wasm((w) => w.live_handles() + w.liveDice());
}
