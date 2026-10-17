// The WASM instance under the API: binding it, and calling into it.
//
// A Rust panic in WASM is a trap (`WebAssembly.RuntimeError: unreachable`): WASM has no unwinding, so the Rust
// frames it abandons run no destructors, and the core's per-call state (recursion levels, the arguments of the
// multiset function bodies being traced) is left behind. So a trap is never handed to the core as a callback's
// exception (callbacks rethrow it): it abandons every WASM frame of the call, and once the outermost one is gone,
// the shadow stack pointer is put back where it was (Rust keeps its locals on a stack in linear memory, and the
// abandoned frames were never popped from it: a few traps would exhaust it), then `recover()` resets that state and
// gives the panic's message, thrown as an `IcepoolError` (`PanicException`, what the Python facade raises for a
// panic). The instance stays usable, and every object stays valid. JS functions and evaluators the abandoned call
// had registered are never released (a leak, not an error).

import type * as Raw from "../pkg-node/icepool_wasm.js";
import { IcepoolError } from "./errors.js";

/** The wasm-bindgen module: its exports (`WasmDie`, `call`, ...), as wasm-pack generates them. */
export type WasmModule = typeof Raw;

let module: WasmModule | null = null;
/** The instance's shadow stack pointer, a `WebAssembly.Global` (see the top of this file). */
let stackPointer: WebAssembly.Global | null = null;
/** What the core calls back (`__icepool_wasm_host`): installed by `bind`, filled by the modules that need it. */
export const HOST: Record<string, unknown> = {};

/** Installs the WASM module; the entry points (node.ts, web.ts) call it once. */
export function bind(wasm: WasmModule): void {
  module = wasm;
  stackPointer = (wasm.stack_pointer() as WebAssembly.Global | undefined) ?? null;
  (globalThis as { __icepool_wasm_host?: unknown }).__icepool_wasm_host = HOST;
}

/** The bound module. */
export function raw(): WasmModule {
  if (module === null) throw new Error("icepool: the WASM module is not bound (import the package's entry point)");
  return module;
}

let depth = 0; // calls into WASM in progress (a JS callback may call in again)
let stackTop = 0; // the shadow stack pointer when the outermost call began

/** `f(module)`, a call into the WASM instance, with a trap turned into an `IcepoolError` once the call is left. */
export function wasm<T>(f: (w: WasmModule) => T): T {
  const w = raw();
  if (depth === 0 && stackPointer) stackTop = stackPointer.value as number;
  depth++;
  try {
    return f(w);
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError && depth === 1) {
      if (stackPointer) stackPointer.value = stackTop;
      const message = w.recover();
      throw new IcepoolError("Panic", "PanicException", message || e.message, { cause: e });
    }
    throw e;
  } finally {
    depth--;
  }
}

/** Whether a callback's exception is a trap below it, which must abandon the WASM frames above it too. */
export function isTrap(e: unknown): boolean {
  return e instanceof WebAssembly.RuntimeError;
}

/** The exports only some builds have: Cargo features `runner` and `khepri` (the npm package has neither, so the
 * generated types the package compiles against lack them). */
interface OptionalExports {
  run?(programJson: string): string;
  calculate?(engineInput: unknown): unknown;
}

/** The raw export `name`, which only builds with the Cargo `feature` have. */
function optional<K extends keyof OptionalExports>(name: K, feature: string): NonNullable<OptionalExports[K]> {
  const f = (raw() as OptionalExports)[name];
  if (typeof f !== "function") {
    throw new Error(`${name}() is not in this build of icepool-wasm: build it with the "${feature}" feature`);
  }
  return f as NonNullable<OptionalExports[K]>;
}

/** One program of the harness's JSON program language (text) in, its JSON result (text) out: what `icepool-run`
 * prints for that line. A trap is a `PanicException`, as for every other call, and the instance stays usable.
 * Test-only: builds with the `runner` feature (`js/build.sh --test`). */
export function run(programJson: string): string {
  const f = optional("run", "runner");
  return wasm(() => f(programJson));
}

/** khepri's result for one engine input (src/khepri.rs): what {@link calculate} returns.
 *
 * @example
 * ```ts
 * import type { KhepriResult } from "icepool-wasm";
 * const result: KhepriResult = { rolls: 400n, outcomes: [{ player: "active", wounds: 1, chance: 0.25 }] };
 * result.outcomes[0].chance; //=> 0.25
 * ```
 */
export interface KhepriResult {
  /** The denominator of the outcomes: a JS number while it is a safe integer, a BigInt beyond 2**53 (as Pyodide's
   * `to_js` converts a Python int). */
  rolls: number | bigint;
  /** One entry per (player, wounds) result; `chance` is `float(Fraction(quantity, denominator))`, bit for bit. */
  outcomes: {
    /** The player the result is for. */
    player: string;
    /** The wounds dealt. */
    wounds: number;
    /** The probability of this result. */
    chance: number;
  }[];
}

/** khepri's `f2f.calculate`: the engine input object in, `{rolls, outcomes: [{player, wounds, chance}, ...]}` out.
 * Throws an `Error` where Python raises, and a `PanicException` after a trap, which leaves the instance usable.
 * Application code, not part of the library: only in builds with the `khepri` feature (khepri's own,
 * crates/icepool-wasm/build.sh). In the npm package's build it throws a plain `Error` saying so.
 *
 * @example
 * ```ts
 * // The npm package is built without the "khepri" feature.
 * calculate({}); //=> throws Error
 * ```
 */
export function calculate(engineInput: object): KhepriResult {
  const f = optional("calculate", "khepri");
  return wasm(() => f(engineInput) as KhepriResult);
}
