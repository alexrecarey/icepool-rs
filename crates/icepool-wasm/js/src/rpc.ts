// Test-only: the JSON-op bridge to the program runner (src/facade.rs, in builds with the `runner` feature), which
// the fixture tests (test/fixtures.mjs, through internal.ts) use for the nodes of the harness's program language
// that the public API has no counterpart for (khepri, complex numbers, the catalogue callables JS lacks, ...). The
// public modules never import this file; the npm package leaves it out (package.json `files`).
//
// Each node is run by the program runner, so it computes exactly what the oracle-checked node computes. Values cross
// as JSON ("JS-JSON", described in src/facade.rs); what has no JSON form stays in WASM memory as a handle. This file
// converts between those and the API's objects: a die, pool, deal, deck or MultiDeal of the API crosses as a handle
// made from its core object, and comes back as an API object; what only the runner models (its expressions, tuple
// expressions, callables, multiset functions) comes back as a `BridgeObject`, which only the bridge can take. A
// native expression with no core object of the runner's (an operator over pools, a mixture, ...) crosses as what
// the `standIn` hook makes of it (the fixture adapter replays the program that made it).

import type { WasmDeck, WasmDie, WasmGenerator, WasmMultiDeal } from "../pkg-node/icepool_wasm.js";
import { Deck, MultiDeal, MultisetTupleExpression } from "./deck.js";
import { Die, DieWithTruth } from "./die.js";
import { makeError, UnsupportedOrder, type ErrorKind } from "./errors.js";
import { MultisetFunction } from "./evaluator.js";
import { generator, generatorOf, MultisetExpression } from "./multiset.js";
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
  type Outcome,
} from "./values.js";

/** The runner's exports: only builds with the `runner` feature have them (the npm package's generated types lack
 * them, so they are typed here). */
interface RunnerExports {
  call(op: string, args: string): string;
  from_wire(literal: string): string;
  wire_result(handle: number): string;
  multiset_function(args: string): string;
  release(handle: number): void;
  live_handles(): number;
  dieHandle(die: WasmDie, truth: boolean | undefined): number;
  deckHandle(deck: WasmDeck): number;
  multiDealHandle(md: WasmMultiDeal): number;
  generatorHandle(g: WasmGenerator): number | undefined;
  WasmDie: { fromHandle(handle: number): WasmDie };
  WasmDeck: { fromHandle(handle: number): WasmDeck };
  WasmMultiDeal: { fromHandle(handle: number): WasmMultiDeal };
  WasmGenerator: { fromHandle(handle: number): WasmGenerator };
}

/** `f` of the runner's exports (a call into WASM, as every other). */
function runner<T>(f: (w: RunnerExports) => T): T {
  return wasm((w) => {
    const r = w as unknown as Partial<RunnerExports>;
    if (typeof r.call !== "function") throw new Error("the bridge is not in this build of icepool-wasm: build it with the \"runner\" feature");
    return f(r as RunnerExports);
  });
}

// --- Objects only the bridge holds --------------------------------------------------------------------------------

/** Frees the handle of a collected object. The registry is the only release for objects nobody frees; the tests
 * free nothing, and the collector frees an object when it likes. */
const registry = new FinalizationRegistry<number>((h) => runner((w) => w.release(h)));

/** What only the runner models: its multiset expressions and tuple expressions, catalogue callables, multiset
 * functions and their raw results (`kind`: the handle's kind). */
export class BridgeObject {
  readonly kind: string;
  private handle: number;

  constructor(kind: string, handle: number) {
    this.kind = kind;
    this.handle = handle;
    registry.register(this, handle, this);
  }

  /** The handle, which must not have been freed. */
  get h(): number {
    if (this.handle < 0) throw new TypeError("icepool: this object was freed");
    return this.handle;
  }

  /** Frees the handle now. */
  free(): void {
    if (this.handle < 0) return;
    const h = this.handle;
    registry.unregister(this);
    this.handle = -1;
    runner((w) => w.release(h));
  }
}

/** Raw JSON text, passed through as it is (the lazy arguments of `khepri` / `mf_def`, in the fixture tests). */
export class RawJSON {
  readonly text: string;
  constructor(text: string) {
    this.text = text;
  }
}

/** Makes the bridge's counterpart of a native expression that has no core object of the runner's (set by the
 * fixture adapter; by default there is none). */
let standIn: (x: object) => unknown = (x) => {
  throw new TypeError(`icepool: the bridge cannot take this ${x.constructor.name}`);
};

/** Sets how a native expression with no core object of the runner's crosses to the bridge. */
export function setStandIn(f: (x: object) => unknown): void {
  standIn = f;
}

// --- JS -> JS-JSON ------------------------------------------------------------------------------------------------

/** A `DieWithTruth` from the bridge whose data raised -> the bridge's handle of it (passed back as it is). */
const failedDice = new WeakMap<DieWithTruth, number>();
const failedRegistry = new FinalizationRegistry<number>((h) => runner((w) => w.release(h)));

/** The bridge's copy of each multiset function it was given (the runner traces it with its own parameters). */
const bridgeFunctions = new WeakMap<MultisetFunction, BridgeObject>();

/** A handle for one call (`temps`, released afterwards), or one the core takes over (`temps` null: a callback's
 * result). */
function handleText(h: number, temps: number[] | null): string {
  if (temps === null) return `{"$take":${h}}`;
  temps.push(h);
  return `{"$h":${h}}`;
}

/** An API object as JS-JSON. */
function encObject(x: object, temps: number[] | null): string {
  if (x instanceof BridgeObject) return `{"$h":${x.h}}`;
  if (x instanceof DieWithTruth) {
    const failed = failedDice.get(x);
    if (failed !== undefined) return `{"$h":${failed}}`;
    const w = x.w;
    return handleText(runner((r) => r.dieHandle(w, x.truth())), temps);
  }
  if (x instanceof Die) {
    const w = x.w;
    return handleText(runner((r) => r.dieHandle(w, undefined)), temps);
  }
  if (x instanceof Deck) {
    const w = x.w;
    return handleText(runner((r) => r.deckHandle(w)), temps);
  }
  if (x instanceof MultisetExpression) {
    const g = generatorOf(x);
    const h = g === undefined ? undefined : runner((r) => r.generatorHandle(g));
    return h === undefined ? enc(standIn(x), temps) : handleText(h, temps);
  }
  if (x instanceof MultisetTupleExpression) {
    const md = x instanceof MultiDeal ? x.md : undefined;
    return md === undefined ? enc(standIn(x), temps) : handleText(runner((r) => r.multiDealHandle(md)), temps);
  }
  if (x instanceof MultisetFunction) {
    let f = bridgeFunctions.get(x);
    if (f === undefined) {
      f = bridgeFunction(x);
      bridgeFunctions.set(x, f);
    }
    return `{"$h":${f.h}}`;
  }
  if (x instanceof List) return `{"$list":[${Array.from(x, (y) => enc(y, temps)).join(",")}]}`;
  if (Array.isArray(x)) return `[${x.map((y) => enc(y, temps)).join(",")}]`;
  if (x instanceof Float) return `{"float":"${pyFloatRepr(x.value)}"}`;
  if (x instanceof Fraction) return `{"frac":["${x.numerator}","${x.denominator}"]}`;
  if (x instanceof Complex) return `{"complex":["${pyFloatRepr(x.re)}","${pyFloatRepr(x.im)}"]}`;
  if (x instanceof Vector) {
    const data = x.items === null ? "null" : `[${x.items.map((y) => enc(y, temps)).join(",")}]`;
    return x.hasTruth ? `{"vector":${data},"truth":${enc(x.truthValue, temps)}}` : `{"vector":${data}}`;
  }
  if (x instanceof Symbols) {
    if (x.items === null) return `{"symbols":${JSON.stringify(x.str)}}`;
    return `{"$symbols":[${x.items.map(([c, n]) => `[${JSON.stringify(c)},${enc(n, temps)}]`).join(",")}],"str":null}`;
  }
  if (x instanceof Special) return `{"$special":"${x.name}"}`;
  if (x instanceof Break) return `{"$break":${enc(x.outcome, temps)}}`;
  if (x instanceof RawJSON) return x.text;
  if (x instanceof StateKey) return `{"$json":${JSON.stringify(x.key)}}`;
  // A Map where Python takes a mapping: a dict, whose keys the core compares with Python's `==` (so `1`, `1.0` and
  // `true` are one key, as in Python; a JS Map keeps them apart).
  if (x instanceof Map) return `{"$map":[${Array.from(x, ([k, v]) => `[${enc(k, temps)},${enc(v, temps)}]`).join(",")}]}`;
  throw new TypeError(`icepool: cannot pass ${Object.prototype.toString.call(x)} to the core`);
}

/** A JS value as JS-JSON; `temps` collects the handles made for one call, released after it (`null`: handles the
 * core takes over, for a callback's result). */
function enc(x: unknown, temps: number[] | null): string {
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
  return x === null ? "null" : encObject(x as object, temps);
}

// --- JS-JSON -> JS ------------------------------------------------------------------------------------------------

/** The API object for a handle of the bridge's (which it takes over, or releases). */
function fromHandle(h: number, kind: string, truth: boolean): unknown {
  // Objects only the bridge models keep their handle.
  if (!["die", "die_truth", "pool", "deal", "deck", "multi_deal"].includes(kind)) return new BridgeObject(kind, h);
  if (kind === "die_truth") {
    let core: WasmDie;
    try {
      core = runner((w) => w.WasmDie.fromHandle(h));
    } catch (e) {
      // Its data raised: the error comes back when the data is used; the bridge gets the handle back.
      const die = DieWithTruth.failedWith(e as never, truth);
      failedDice.set(die, h);
      failedRegistry.register(die, h);
      return die;
    }
    runner((w) => w.release(h));
    return DieWithTruth.wrapTruth(core, truth);
  }
  try {
    return runner((w) => {
      switch (kind) {
        case "die":
          return Die.wrap(w.WasmDie.fromHandle(h));
        case "deck":
          return Deck.wrap(w.WasmDeck.fromHandle(h));
        case "multi_deal":
          return MultiDeal.native({ kind: "multiDeal", md: w.WasmMultiDeal.fromHandle(h) });
        default:
          return generator(w.WasmGenerator.fromHandle(h));
      }
    });
  } finally {
    runner((w) => w.release(h));
  }
}

/** A float from its Python repr. */
const floatOf = (s: string) => Number(s === "inf" ? Infinity : s === "-inf" ? -Infinity : s);

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

function dec(j: Json): unknown {
  if (j === null || typeof j !== "object") return j;
  if (Array.isArray(j)) return j.map(dec);
  if ("$h" in j) return fromHandle(j.$h as number, j.kind as string, j.truth === true);
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

/** Calls a raw export: `f` encodes the arguments (collecting the handles made for them in `temps`, released
 * afterwards) and calls it; its result envelope is decoded. */
function callWith(f: (temps: number[]) => string): unknown {
  const temps: number[] = [];
  try {
    return dec(unwrap(runner(() => f(temps))));
  } finally {
    for (const h of temps) runner((w) => w.release(h));
  }
}

/** How many times each node was called through the bridge (the fixture test reports it: the nodes the public API
 * has no counterpart for). */
export const bridgeCalls = new Map<string, number>();

function countBridgeCall(name: string): void {
  bridgeCalls.set(name, (bridgeCalls.get(name) ?? 0) + 1);
}

/** Calls a node of the program language by name: `call("add", { a: d(6), b: 1 })`. */
export function call(op: string, args: Record<string, unknown> = {}): unknown {
  countBridgeCall(op);
  return callWith((temps) => {
    const parts = Object.entries(args).map(([k, v]) => `${JSON.stringify(k)}:${enc(v, temps)}`);
    const text = `{${parts.join(",")}}`;
    return runner((w) => w.call(op, text));
  });
}

/** A literal of the harness's wire format (`{"float": "1.5"}`, `{"tuple": [...]}`, ...) as a JS value, read
 * exactly as the program runner reads it. */
export function fromWire(text: string): unknown {
  return dec(unwrap(runner((w) => w.from_wire(text))));
}

/** The harness's wire encoding of a value only the bridge's encoder knows (`{"ok": true, "die" | "value": ...}`,
 * JSON text): a pool, an expression, a callable... */
export function wireResult(x: object): string {
  const temps: number[] = [];
  try {
    const { $h } = JSON.parse(enc(x, temps)) as { $h: number };
    return runner((w) => w.wire_result($h));
  } finally {
    for (const h of temps) runner((w) => w.release(h));
  }
}

/** How many core objects JS holds through the bridge (for leak checks). */
export function liveHandles(): number {
  return runner((w) => w.live_handles());
}

/** The bridge's copy of a native multiset function: the runner traces the same JS function with its own
 * parameters. */
function bridgeFunction(fn: MultisetFunction): BridgeObject {
  const { f, params, rest, kwonly } = fn.traced;
  countBridgeCall("multiset_function");
  return callWith((temps) => {
    const text = `{"f":${enc(f, temps)},"params":${params},"var":${rest ? "true" : "false"},"kwonly":${JSON.stringify(kwonly)}}`;
    return runner((w) => w.multiset_function(text));
  }) as BridgeObject;
}

// --- Callbacks from the core --------------------------------------------------------------------------------------

/** id -> JS function; one id per time a function is passed, released by the core. */
const functions = new Map<number, (...args: never[]) => unknown>();
let nextId = 1;

function registerFunction(f: (...args: never[]) => unknown): number {
  const id = nextId++;
  functions.set(id, f);
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
  },
});
