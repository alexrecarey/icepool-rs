// The trap recovery of the TypeScript API (js/src/wasm.ts `wasm`), for the harness's own drivers of a raw wasm-pack
// build (run-wasm.mjs, test-calculate.mjs), which do not build the TypeScript package. A Rust panic is a trap in
// WASM (no unwinding): `guarded(f)` puts the shadow stack pointer back where it was before the call, calls
// `recover()` to reset the core's per-call state, and throws the panic's message as a `Panic` error, leaving the
// instance usable. See js/src/wasm.ts for the full story; keep the two in step.

/** A Rust panic, after recovery. */
export class Panic extends Error {
  constructor(message, options) {
    super(message, options);
    this.name = "PanicException";
  }
}

/** `{ run, calculate }` of the wasm-pack module `w`, each recovering from a trap. */
export function bindRaw(w) {
  const stackPointer = w.stack_pointer() ?? null;
  const guarded = (f) => (...args) => {
    const top = stackPointer?.value;
    try {
      return f(...args);
    } catch (e) {
      if (!(e instanceof WebAssembly.RuntimeError)) throw e;
      if (stackPointer) stackPointer.value = top;
      throw new Panic(w.recover() || e.message, { cause: e });
    }
  };
  return { run: w.run && guarded(w.run), calculate: w.calculate && guarded(w.calculate) };
}
