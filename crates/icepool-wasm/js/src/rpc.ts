// INTERIM (task C6, see NOTES.md): the JSON-op bridge to the program runner (src/facade.rs) for the parts of the
// API that have not moved onto the typed exports yet (decks and deals, the keep family of Die...).
// Each such method is one node of the harness's program language (harness/grammar/), run by the core's program
// layer, so it computes exactly what the oracle-checked node computes. This file goes away when the last of them
// moves (task C6l).
//
// Values cross as JSON ("JS-JSON", described in src/facade.rs); pools, decks and expressions stay in WASM memory as
// handles, released when their JS object is garbage-collected, or at once by `free()` / `Symbol.dispose`. A die of
// the typed API crosses as a handle made for the call (`dieHandle`), and a die handle coming back becomes a typed
// die (`WasmDie.fromHandle`).

import { makeError, UnsupportedOrder, type ErrorKind } from "./errors.js";
import { HOST, isTrap, wasm } from "./wasm.js";
import {
  Break,
  Complex,
  Float,
  Fraction,
  List,
  Opaque,
  Special,
  Symbols,
  Vector,
  pyFloatRepr,
  Reroll,
  Restart,
  Ellipsis,
  StateKey,
  stateValue,
  type Outcome,
} from "./values.js";

/** Encodes an object the bridge does not know about (a die of the typed API) as JS-JSON. `temps` collects handles
 * made for one call, released after it; `null` asks for a handle the core takes over (a callback's result). */
export const ENCODE = Symbol("icepool.encode");
export interface Encodable {
  [ENCODE](temps: number[] | null): string;
}
const isEncodable = (x: object): x is Encodable => ENCODE in x;

/** Raw JSON text, passed through as it is (the lazy arguments of `khepri` / `mf_def`, in the fixture tests). */
export class RawJSON {
  readonly text: string;
  constructor(text: string) {
    this.text = text;
  }
}

// --- JS -> JS-JSON ------------------------------------------------------------------------------------------------

export function enc(x: unknown, temps: number[] | null): string {
  switch (typeof x) {
    case "number":
      return Number.isInteger(x) ? BigInt(x).toString() : `{"float":"${pyFloatRepr(x)}"}`;
    case "bigint":
      return x.toString();
    case "string":
      return JSON.stringify(x);
    case "boolean":
      return x ? "true" : "false";
    case "undefined":
      return "null";
    case "function":
      return `{"$fn":${registerFunction(x as (...args: never[]) => unknown)},"n":${x.length}}`;
  }
  if (x === null) return "null";
  const e = (y: unknown) => enc(y, temps);
  if (isEncodable(x as object)) return (x as Encodable)[ENCODE](temps);
  if (x instanceof List) return `{"$list":[${Array.from(x, e).join(",")}]}`;
  if (Array.isArray(x)) return `[${x.map(e).join(",")}]`;
  if (x instanceof Float) return `{"float":"${pyFloatRepr(x.value)}"}`;
  if (x instanceof Fraction) return `{"frac":["${x.numerator}","${x.denominator}"]}`;
  if (x instanceof Complex) return `{"complex":["${pyFloatRepr(x.re)}","${pyFloatRepr(x.im)}"]}`;
  if (x instanceof Vector) {
    const data = x.items === null ? "null" : `[${x.items.map(e).join(",")}]`;
    return x.hasTruth ? `{"vector":${data},"truth":${e(x.truthValue)}}` : `{"vector":${data}}`;
  }
  if (x instanceof Symbols) {
    if (x.items === null) return `{"symbols":${JSON.stringify(x.str)}}`;
    return `{"$symbols":[${x.items.map(([c, n]) => `[${JSON.stringify(c)},${e(n)}]`).join(",")}],"str":null}`;
  }
  if (x instanceof Special) return `{"$special":"${x.name}"}`;
  if (x instanceof Break) return `{"$break":${e(x.outcome)}}`;
  if (x instanceof RawJSON) return x.text;
  if (x instanceof StateKey) return `{"$json":${JSON.stringify(x.key)}}`;
  // A Map where Python takes a mapping (`map`, `reroll`, `groupBy`, ...): a dict, whose keys the core compares
  // with Python's `==` (so `1`, `1.0` and `true` are one key, as in Python; a JS Map keeps them apart).
  if (x instanceof Map) return `{"$map":[${Array.from(x, ([k, v]) => `[${e(k)},${e(v)}]`).join(",")}]}`;
  throw new TypeError(`icepool: cannot pass ${Object.prototype.toString.call(x)} to the core`);
}

// --- JS-JSON -> JS ------------------------------------------------------------------------------------------------

/** Makes the JS object for a handle of a kind (`die`, `pool`, ...); the modules defining them register here. */
type Maker = (handle: number, truth: boolean) => unknown;
const KINDS = new Map<string, Maker>();
export function registerKind(kind: string, make: Maker): void {
  KINDS.set(kind, make);
}

/** A float from its Python repr. */
const floatOf = (s: string) => Number(s === "inf" ? Infinity : s === "-inf" ? -Infinity : s);

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export function dec(j: Json): unknown {
  if (j === null || typeof j !== "object") return j;
  if (Array.isArray(j)) return j.map(dec);
  if ("$h" in j) {
    const make = KINDS.get(j.kind as string);
    if (!make) throw new TypeError(`icepool: unknown kind of core object ${String(j.kind)}`);
    return make(j.$h as number, j.truth === true);
  }
  if ("$int" in j) return BigInt(j.$int as string);
  if ("float" in j) {
    const x = floatOf(j.float as string);
    return Number.isInteger(x) ? new Float(x) : x;
  }
  if ("frac" in j) {
    const [n, d] = j.frac as string[];
    return new Fraction(BigInt(n), BigInt(d));
  }
  if ("$list" in j) return List.from((j.$list as Json[]).map(dec));
  if ("$special" in j) return ({ Reroll, Restart, Ellipsis } as Record<string, Special>)[j.$special as string];
  if ("$break" in j) return new Break(dec(j.$break) as Outcome);
  if ("$json" in j) return JSON.parse(j.$json as string) as unknown;
  if ("vector" in j) {
    const items = j.vector === null ? null : (j.vector as Json[]).map(dec);
    return Vector.fromCore(items as Outcome[] | null, "truth" in j, dec(j.truth ?? null) as Outcome);
  }
  if ("$symbols" in j) {
    const items = (j.$symbols as [string, Json][]).map(([c, n]) => [c, dec(n)] as const);
    return Symbols.fromCore(items as [string, bigint][], j.str as string | null);
  }
  if ("complex" in j) {
    const [re, im] = (j.complex as string[]).map(floatOf);
    return new Complex(re, im);
  }
  if ("repr" in j) return new Opaque(j.repr as string);
  throw new TypeError(`icepool: unexpected value from the core: ${JSON.stringify(j)}`);
}

// --- Calling the core ---------------------------------------------------------------------------------------------

let pendingError: unknown; // the exception a JS callback threw, rethrown when the core returns

function unwrap(text: string): Json {
  const r = JSON.parse(text) as { [key: string]: Json };
  if ("ok" in r) return r.ok;
  if (r.host) {
    const e = pendingError;
    pendingError = undefined;
    throw e;
  }
  const kind = (r.kind as ErrorKind | undefined) ?? (r.unimplemented === true ? "Unported" : "WrongType");
  // makeError, so the subclass (`ConflictingOrderError`, ...) survives the bridge as it does a native call.
  throw makeError(kind, r.error as string, r.message as string);
}

function argsText(args: Record<string, unknown>, temps: number[]): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(args)) parts.push(`${JSON.stringify(k)}:${enc(v, temps)}`);
  return `{${parts.join(",")}}`;
}

/** Calls a raw export: `f` encodes the arguments (collecting the handles made for them in `temps`, released
 * afterwards) and calls it; its result envelope is decoded. */
export function callWith(f: (temps: number[]) => string): unknown {
  const temps: number[] = [];
  try {
    return dec(unwrap(wasm(() => f(temps))));
  } finally {
    for (const h of temps) wasm((w) => w.release(h));
  }
}

/** How many times each node was called through the bridge (the JS tests report it: what is left to move onto
 * typed exports). */
export const bridgeCalls = new Map<string, number>();

/** Counts a call through the bridge: a node, or another of its exports (`evaluate`, `multiset_function`). */
export function countBridgeCall(name: string): void {
  bridgeCalls.set(name, (bridgeCalls.get(name) ?? 0) + 1);
}

/** Calls a node of the program language by name: `call("add", { a: d(6), b: 1 })`. */
export function call(op: string, args: Record<string, unknown> = {}): unknown {
  countBridgeCall(op);
  return callWith((temps) => {
    const text = argsText(args, temps);
    return wasm((w) => w.call(op, text));
  });
}

/** A literal of the harness's wire format (`{"float": "1.5"}`, `{"tuple": [...]}`, ...) as a JS value, read
 * exactly as the program runner reads it (for the fixture tests). */
export function fromWire(text: string): unknown {
  return dec(unwrap(wasm((w) => w.from_wire(text))));
}

/** JS-JSON text of a call's keyword arguments. */
export function kwargsText(kwargs: Record<string, unknown>, temps: number[]): string {
  return argsText(kwargs, temps);
}

// --- Callbacks from the core --------------------------------------------------------------------------------------

/** id -> JS function; one id per time a function is passed, released by the core. */
const functions = new Map<number, (...args: never[]) => unknown>();
/** id -> evaluator, from one evaluate() until the core releases it (at once, or later for an evaluation recorded
 * in a multiset function's body). */
const evaluators = new Map<number, EvaluatorMethods>();
let nextId = 1;

function registerFunction(f: (...args: never[]) => unknown): number {
  const id = nextId++;
  functions.set(id, f);
  return id;
}

/** The methods of a JS evaluator, as the core calls them (evaluator.ts's `MultisetEvaluator`). */
export interface EvaluatorMethods {
  nextState(state: unknown, order: number, outcome: unknown, ...counts: unknown[]): unknown;
  initialState?(order: number, outcomes: unknown[], ...rest: unknown[]): unknown;
  finalOutcome?(finalState: unknown, order: number, outcomes: unknown[], ...rest: unknown[]): unknown;
  extraOutcomes?(outcomes: unknown[]): Iterable<unknown>;
}

/** Registers an evaluator for one `evaluate()`; the core releases it. */
export function registerEvaluator(ev: EvaluatorMethods): number {
  const id = nextId++;
  evaluators.set(id, ev);
  return id;
}

/** Runs a JS callback for the core: a result envelope, exceptions kept here and rethrown when the core returns. */
function callback(body: () => string): string {
  try {
    return `{"ok":${body()}}`;
  } catch (e) {
    // A trap in a nested call: the WASM frames below must be abandoned too (see wasm.ts).
    if (isTrap(e)) throw e;
    if (e instanceof UnsupportedOrder) return `{"error":"UnsupportedOrder"}`;
    pendingError = e;
    return `{"throw":true}`;
  }
}

/** A callback's keyword arguments (JS-JSON object text) as an options object. */
function kwargsOf(text: string): Record<string, unknown> {
  const raw = JSON.parse(text) as Record<string, Json>;
  return Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, dec(v)]));
}

Object.assign(HOST, {
  call(id: number, args: string, kwargs: string): string {
    return callback(() => {
      const f = functions.get(id) as (...args: unknown[]) => unknown;
      const a = dec(JSON.parse(args) as Json) as unknown[];
      const kw = kwargsOf(kwargs);
      // Python passes **kwargs; JS gets them as a trailing options object.
      const result = Object.keys(kw).length ? f(...a, kw) : f(...a);
      return enc(result, null);
    });
  },
  release(id: number): void {
    functions.delete(id);
    evaluators.delete(id);
  },
  evaluator(id: number, method: string, args: string, kwargs: string): string {
    return callback(() => {
      const ev = evaluators.get(id) as EvaluatorMethods;
      const a = dec(JSON.parse(args) as Json) as unknown[];
      // evaluate()'s keyword arguments go to initialState and finalOutcome, as a trailing options object.
      const kw = kwargsOf(kwargs);
      if (Object.keys(kw).length && (method === "initialState" || method === "finalOutcome")) a.push(kw);
      switch (method) {
        case "initialState":
          return enc(stateValue(ev.initialState ? ev.initialState(...(a as [number, unknown[]])) : null), null);
        case "nextState": {
          const next = ev.nextState(...(a as [unknown, number, unknown]));
          return enc(next === Reroll ? next : stateValue(next), null);
        }
        case "extraOutcomes":
          return enc(ev.extraOutcomes ? Array.from(ev.extraOutcomes(...(a as [unknown[]]))) : [], null);
        case "finalOutcome":
          // By default the final outcome is the final state.
          return enc(ev.finalOutcome ? ev.finalOutcome(...(a as [unknown, number, unknown[]])) : a[0], null);
      }
      throw new Error(`unknown evaluator method ${method}`);
    });
  },
});

// --- Handles ------------------------------------------------------------------------------------------------------

/** Frees the handle of a collected object. The registry is the only release for objects nobody frees, but the
 * collector frees an object when it likes (in a loop that makes many, the WASM memory grows meanwhile), and never
 * frees one a JS function the core holds refers to: such a function (a `mapCounts` callback, a multiset function)
 * sits in `functions` until the core drops it, and the core drops it only when the handle is released. Hence
 * `free()`. */
const registry = new FinalizationRegistry<number>((h) => wasm((w) => w.release(h)));

/** The constructor token: wrap an existing handle. */
export const HANDLE: unique symbol = Symbol("handle");

/** A core object (pool, deck, expression, ...) held by JS through the bridge. */
export class Handle implements Disposable {
  /** The handle; -1 once freed. @internal */
  protected handle: number;

  constructor(token: typeof HANDLE, handle: number) {
    if (token !== HANDLE) throw new TypeError("icepool: use the factory functions to make core objects");
    this.handle = handle;
    // -1: a constructor about to adopt a handle (see `adopt`).
    if (handle >= 0) registry.register(this, handle, this);
  }

  /** The handle, which must not have been freed. @internal */
  protected get h(): number {
    if (this.handle < 0) throw new TypeError("icepool: this object was freed");
    return this.handle;
  }

  /** @internal */
  [ENCODE](): string {
    return `{"$h":${this.h}}`;
  }

  /** The harness's wire encoding of this object (`{"ok": true, "die" | "value": ...}`), as JSON text (for the
   * fixture tests). @internal */
  toWire(): string {
    const h = this.h;
    return wasm((w) => w.wire_result(h));
  }

  /** Frees the object's WASM memory now rather than when it is garbage-collected. Using it afterwards throws a
   * `TypeError`; freeing it again does nothing. Objects made from it (`pool.sum()`) are not affected. */
  free(): void {
    if (this.handle < 0) return;
    const h = this.handle;
    registry.unregister(this);
    this.handle = -1;
    wasm((w) => w.release(h));
  }

  /** `free()`, for `using pool = d(6).pool(3);`. */
  [Symbol.dispose](): void {
    this.free();
  }

  /** Adopts the handle of a freshly made object of class `cls` into this one (for `new Pool(...)` and friends,
   * whose work is a node call). @internal */
  protected adopt(made: unknown, cls: abstract new (...args: never[]) => Handle): void {
    if (!(made instanceof cls)) throw new TypeError(`icepool: expected a ${cls.name}`);
    registry.unregister(made);
    this.handle = made.handle;
    made.handle = -1;
    registry.register(this, this.handle, this);
  }
}

/** An object of a class whose constructor makes a new core object (`Pool`, `Deck`), around an existing handle. */
export function wrapHandle<T extends Handle>(proto: T, h: number): T {
  const made = Object.create(proto) as T;
  (made as unknown as { handle: number }).handle = h;
  registry.register(made, h, made);
  return made;
}

/** How many core objects JS holds through the bridge (for leak checks). */
export function liveHandles(): number {
  return wasm((w) => w.live_handles());
}
