// The JS facade over the Rust core: icepool's API as JS classes and methods. Arithmetic is methods (`d(6).add(1)`,
// `die.gt(10)`), keyword arguments are a trailing options object (`die.reroll([1], { depth: 2 })`), quantities are
// BigInts. Every method is one node of the harness's program language (harness/grammar/), run by the core's program
// layer, so the JS API computes exactly what the oracle-checked nodes compute.
//
// Values cross the WASM boundary as JSON ("JS-JSON", described in src/facade.rs); dice, pools, decks and expressions
// stay in WASM memory as handles, released when their JS object is garbage-collected, or at once by `release()` /
// `Symbol.dispose` (`using die = d(6)`).
//
// Python types without a JS counterpart:
//   int       a JS number when it is a safe integer, else a BigInt (both go in as ints)
//   float     a JS number when it is not integral; an integral float (2.0, -0.0) comes out as a `Float`, so that it
//             goes back in as a float. Any non-integral number, Infinity and NaN go in as floats.
//   tuple     a JS array (a Python list, where an argument must be one, is a `List`)
//   Fraction, Vector, Symbols: classes below. None is null.

let W = null; // the wasm-bindgen module (bind())
let SP = null; // the instance's shadow stack pointer, a `WebAssembly.Global` (see `wasm`)

/** Installs the WASM module; node.js and web.js call it once. */
export function bind(wasm) {
  W = wasm;
  SP = wasm.stack_pointer() ?? null;
  globalThis.__icepool_wasm_host = HOST;
}

// --- Errors -------------------------------------------------------------------------------------------------------

/** An exception icepool raised; `pythonType` is the Python exception's name (`ValueError`, `TypeError`, ...). */
export class IcepoolError extends Error {
  constructor(pythonType, message, unimplemented = false) {
    super(`${pythonType}: ${message ?? ""}`);
    this.name = pythonType;
    this.pythonType = pythonType;
    /** True when Python would answer but the port does not cover the case yet. */
    this.unimplemented = unimplemented;
  }
}

// A Rust panic in WASM is a trap (`WebAssembly.RuntimeError: unreachable`): WASM has no unwinding, so the Rust frames
// it abandons run no destructors, and the core's per-call state (recursion levels, the arguments of the multiset
// function bodies being traced) is left behind. So a trap is never handed to the core as a callback's exception
// (`callback` rethrows it): it abandons every WASM frame of the call, and once the outermost one is gone,
// the shadow stack pointer is put back where it was (Rust keeps its locals on a stack in linear memory, and the
// abandoned frames were never popped from it: a few traps would exhaust it), then `W.recover()` resets that state
// and gives the panic's message, thrown as an `IcepoolError` (`PanicException`, what the Python facade raises for a
// panic). The instance stays usable, and handles stay valid. JS functions and evaluators the abandoned call had
// registered are never released (a leak, not an error).
let depth = 0; // calls into WASM in progress (a JS callback may call in again)
let stackTop = 0; // the shadow stack pointer when the outermost call began

/** `f()`, a call into the WASM instance, with a trap turned into an `IcepoolError` once the call is left. */
function wasm(f) {
  if (depth === 0 && SP) stackTop = SP.value;
  depth++;
  try {
    return f();
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError && depth === 1) {
      if (SP) SP.value = stackTop;
      const message = W.recover();
      const error = new IcepoolError("PanicException", message || e.message);
      error.cause = e;
      throw error;
    }
    throw e;
  } finally {
    depth--;
  }
}

/** Thrown by an evaluator's method when it cannot handle the order it is given: the evaluation retries in the
 * other order (icepool's `UnsupportedOrder`). */
export class UnsupportedOrder extends Error {
  constructor(message = "") {
    super(message);
    this.name = "UnsupportedOrder";
  }
}

// --- Value types --------------------------------------------------------------------------------------------------

/** A Python float that happens to be integral (`2.0`), so that it stays a float. Behaves as its number. */
export class Float {
  constructor(value) {
    this.value = Number(value);
  }
  valueOf() {
    return this.value;
  }
  toString() {
    return pyFloatRepr(this.value);
  }
}

/** An exact fraction (Python's `Fraction`), normalised by the core. */
export class Fraction {
  constructor(numerator, denominator = 1n) {
    this.numerator = BigInt(numerator);
    this.denominator = BigInt(denominator);
  }
  valueOf() {
    return Number(this.numerator) / Number(this.denominator);
  }
  toString() {
    return this.denominator === 1n ? `${this.numerator}` : `${this.numerator}/${this.denominator}`;
  }
}

/** A Python list, where an argument must be a list rather than a tuple (a JS array is a tuple). */
export class List extends Array {}

/** `icepool.Break(outcome)`, returned by a `map` function to stop the repetition. */
export class Break {
  constructor(outcome = null) {
    this.outcome = outcome;
  }
}

/** Python's `complex`: `re` and `im` are numbers; `String(z)` is Python's repr, e.g. `(1+2j)`, `-0j`. */
export class Complex {
  constructor(re, im = 0) {
    this.re = re;
    this.im = im;
  }
  toString() {
    return pyComplexRepr(this.re, this.im);
  }
}

/** A Python object JS has no type for: it can be read, not passed back. */
export class Opaque {
  constructor(repr) {
    this.repr = repr;
  }
  toString() {
    return this.repr;
  }
}

class Special {
  constructor(name) {
    this.name = name;
  }
  toString() {
    return this.name;
  }
}
/** `icepool.Reroll`, `icepool.Restart` and `...` (Ellipsis, for keep indices). */
export const Reroll = new Special("Reroll");
export const Restart = new Special("Restart");
export const Ellipsis = new Special("Ellipsis");
const SPECIALS = { Reroll, Restart, Ellipsis };

/** `icepool.Order`. */
export const Order = Object.freeze({ Ascending: 1, Descending: -1, Any: 0 });

/** Python's float repr: shortest round-trip digits, exponent form below 1e-4 or from 1e16 (CPython's 'r'). */
export function pyFloatRepr(x) {
  if (Number.isNaN(x)) return "nan";
  if (!Number.isFinite(x)) return x > 0 ? "inf" : "-inf";
  if (x === 0) return Object.is(x, -0) ? "-0.0" : "0.0";
  const [mant, e] = x.toExponential().split("e");
  const exp = Number(e);
  const sign = mant.startsWith("-") ? "-" : "";
  const digits = mant.replace("-", "").replace(".", "");
  if (exp < -4 || exp >= 16) {
    const m = digits.length > 1 ? `${digits[0]}.${digits.slice(1)}` : digits;
    return `${sign}${m}e${exp < 0 ? "-" : "+"}${String(Math.abs(exp)).padStart(2, "0")}`;
  }
  if (exp < 0) return `${sign}0.${"0".repeat(-exp - 1)}${digits}`;
  const int = digits.slice(0, exp + 1).padEnd(exp + 1, "0");
  const frac = digits.slice(exp + 1);
  return `${sign}${int}.${frac || "0"}`;
}

/** Python's `repr(complex)`: `(re+imj)`, or just `imj` when the real part is +0.0; no trailing `.0`. */
export function pyComplexRepr(re, im) {
  const short = (x) => pyFloatRepr(x).replace(/\.0$/, "");
  if (re === 0 && !Object.is(re, -0)) return `${short(im)}j`;
  const i = short(im);
  return `(${short(re)}${i.startsWith("-") ? "" : "+"}${i}j)`;
}

// --- JS -> JS-JSON ------------------------------------------------------------------------------------------------

/** Raw JSON text, passed through as it is (the lazy arguments of `khepri` / `mf_def`, in the fixture tests). */
export class RawJSON {
  constructor(text) {
    this.text = text;
  }
}

function enc(x) {
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
      return `{"$fn":${registerFunction(x)},"n":${x.length}}`;
  }
  if (x === null) return "null";
  if (x instanceof Handle) return `{"$h":${handleOf(x)}}`;
  if (x instanceof List) return `{"$list":[${Array.from(x, enc).join(",")}]}`;
  if (Array.isArray(x)) return `[${x.map(enc).join(",")}]`;
  if (x instanceof Float) return `{"float":"${pyFloatRepr(x.value)}"}`;
  if (x instanceof Fraction) return `{"frac":["${x.numerator}","${x.denominator}"]}`;
  if (x instanceof Complex) return `{"complex":["${pyFloatRepr(x.re)}","${pyFloatRepr(x.im)}"]}`;
  if (x instanceof Vector) {
    const data = x.items === null ? "null" : `[${x.items.map(enc).join(",")}]`;
    return x.hasTruth ? `{"vector":${data},"truth":${enc(x.truthValue)}}` : `{"vector":${data}}`;
  }
  if (x instanceof Symbols) {
    if (x._items === null) return `{"symbols":${JSON.stringify(x.str)}}`;
    return `{"$symbols":[${x._items.map(([c, n]) => `[${JSON.stringify(c)},${enc(n)}]`).join(",")}],"str":null}`;
  }
  if (x instanceof Special) return `{"$special":"${x.name}"}`;
  if (x instanceof Break) return `{"$break":${enc(x.outcome)}}`;
  if (x instanceof RawJSON) return x.text;
  if (x instanceof StateKey) return `{"$json":${JSON.stringify(x.key)}}`;
  // A Map where Python takes a mapping (`map`, `reroll`, `groupBy`, ...): a dict, whose keys the core compares
  // with Python's `==` (so `1`, `1.0` and `true` are one key, as in Python; a JS Map keeps them apart).
  if (x instanceof Map) return `{"$map":[${Array.from(x, ([k, v]) => `[${enc(k)},${enc(v)}]`).join(",")}]}`;
  throw new TypeError(`icepool: cannot pass ${Object.prototype.toString.call(x)} to the core`);
}

/** An evaluator state that is a plain JS object: keyed (hashed, compared) by its canonical JSON. */
class StateKey {
  constructor(key) {
    this.key = key;
  }
}

function canonicalJSON(x) {
  if (Array.isArray(x)) return `[${x.map(canonicalJSON).join(",")}]`;
  if (x !== null && typeof x === "object") {
    const keys = Object.keys(x).sort();
    return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJSON(x[k])}`).join(",")}}`;
  }
  if (typeof x === "bigint") throw new TypeError("icepool: a BigInt inside an object state is not serialisable");
  return JSON.stringify(x);
}

/** An evaluator state as a value: plain objects become keys (anywhere inside arrays too); the rest as is. */
function stateValue(x) {
  if (Array.isArray(x)) return x.map(stateValue);
  if (x !== null && typeof x === "object" && Object.getPrototypeOf(x) === Object.prototype) {
    return new StateKey(canonicalJSON(x));
  }
  return x;
}

// --- JS-JSON -> JS ------------------------------------------------------------------------------------------------

const KINDS = {}; // handle kind -> class, filled in below

/** A float from its Python repr. */
const floatOf = (s) => Number(s === "inf" ? Infinity : s === "-inf" ? -Infinity : s);

function dec(j) {
  if (j === null || typeof j !== "object") return j;
  if (Array.isArray(j)) return j.map(dec);
  if ("$h" in j) {
    if (j.kind === "die_truth") return new DieWithTruth(HANDLE, j.$h, j.truth);
    return new KINDS[j.kind](HANDLE, j.$h);
  }
  if ("$int" in j) return BigInt(j.$int);
  if ("float" in j) {
    const x = floatOf(j.float);
    return Number.isInteger(x) ? new Float(x) : x;
  }
  if ("frac" in j) return new Fraction(BigInt(j.frac[0]), BigInt(j.frac[1]));
  if ("$list" in j) return List.from(j.$list.map(dec));
  if ("$special" in j) return SPECIALS[j.$special];
  if ("$break" in j) return new Break(dec(j.$break));
  if ("$json" in j) return JSON.parse(j.$json);
  if ("vector" in j) return Vector._make(j.vector === null ? null : j.vector.map(dec), "truth" in j, dec(j.truth ?? null));
  if ("$symbols" in j) return Symbols._make(j.$symbols.map(([c, n]) => [c, dec(n)]), j.str);
  if ("complex" in j) return new Complex(...j.complex.map(floatOf));
  if ("repr" in j) return new Opaque(j.repr);
  throw new TypeError(`icepool: unexpected value from the core: ${JSON.stringify(j)}`);
}

// --- Calling the core ---------------------------------------------------------------------------------------------

let pendingError; // the exception a JS callback threw, rethrown when the core returns

function unwrap(text) {
  const r = JSON.parse(text);
  if ("ok" in r) return r.ok;
  if (r.host) {
    const e = pendingError;
    pendingError = undefined;
    throw e;
  }
  throw new IcepoolError(r.error, r.message, r.unimplemented === true);
}

function argsText(args) {
  const parts = [];
  for (const [k, v] of Object.entries(args)) parts.push(`${JSON.stringify(k)}:${enc(v)}`);
  return `{${parts.join(",")}}`;
}

/** Calls a node of the program language by name: `call("add", { a: d(6), b: 1 })`. The methods below are all
 * built on this; it is exported for the nodes without a method (and for the fixture tests). */
export function call(op, args = {}) {
  return dec(unwrap(wasm(() => W.call(op, argsText(args)))));
}

/** A literal of the harness's wire format (`{"float": "1.5"}`, `{"tuple": [...]}`, ...) as a JS value, read
 * exactly as the program runner reads it (for the fixture tests). */
export function fromWire(text) {
  return dec(unwrap(wasm(() => W.from_wire(text))));
}

/** How many core objects JS holds (for leak checks). */
export function liveHandles() {
  return wasm(() => W.live_handles());
}

/** The raw export `name`, which only builds with the Cargo `feature` have (the npm package has neither). */
function optional(name, feature) {
  if (typeof W[name] !== "function") {
    throw new Error(`${name}() is not in this build of icepool-wasm: build it with the "${feature}" feature`);
  }
  return W[name];
}

/** One program of the harness's JSON program language (text) in, its JSON result (text) out: what `icepool-run`
 * prints for that line. A trap is a `PanicException`, as for every other call, and the instance stays usable.
 * Test-only: builds with the `runner` feature (run-wasm.sh, `js/build.sh --test`). */
export function run(programJson) {
  const f = optional("run", "runner");
  return wasm(() => f(programJson));
}

/** khepri's `f2f.calculate`: the engine input object in, `{rolls, outcomes: [{player, wounds, chance}, ...]}` out
 * (src/lib.rs). Throws an `Error` where Python raises, and a `PanicException` after a trap, which leaves the
 * instance usable: call this rather than the raw wasm-bindgen export. Only in builds with the `khepri` feature
 * (khepri's own, crates/icepool-wasm/build.sh). */
export function calculate(engineInput) {
  const f = optional("calculate", "khepri");
  return wasm(() => f(engineInput));
}

// --- Callbacks from the core --------------------------------------------------------------------------------------

const functions = new Map(); // id -> JS function; one id per time a function is passed, released by the core
// id -> evaluator, from one evaluate() until the core releases it (at once, or later for an evaluation recorded in
// a multiset function's body)
const evaluators = new Map();
let nextId = 1;

function registerFunction(f) {
  const id = nextId++;
  functions.set(id, f);
  return id;
}

/** Runs a JS callback for the core: a result envelope, exceptions kept here and rethrown when the core returns. */
function callback(body) {
  try {
    return `{"ok":${body()}}`;
  } catch (e) {
    // A trap in a nested call: the WASM frames below must be abandoned too (see `wasm`).
    if (e instanceof WebAssembly.RuntimeError) throw e;
    if (e instanceof UnsupportedOrder) return `{"error":"UnsupportedOrder"}`;
    pendingError = e;
    return `{"throw":true}`;
  }
}

const HOST = {
  call(id, args, kwargs) {
    return callback(() => {
      const f = functions.get(id);
      const a = dec(JSON.parse(args));
      const kw = Object.fromEntries(Object.entries(JSON.parse(kwargs)).map(([k, v]) => [k, dec(v)]));
      // Python passes **kwargs; JS gets them as a trailing options object.
      const result = Object.keys(kw).length ? f(...a, kw) : f(...a);
      return enc(result);
    });
  },
  release(id) {
    functions.delete(id);
    evaluators.delete(id);
  },
  evaluator(id, method, args, kwargs) {
    return callback(() => {
      const ev = evaluators.get(id);
      const a = dec(JSON.parse(args));
      // evaluate()'s keyword arguments go to initialState and finalOutcome, as a trailing options object.
      const kw = Object.fromEntries(Object.entries(JSON.parse(kwargs)).map(([k, v]) => [k, dec(v)]));
      if (Object.keys(kw).length && (method === "initialState" || method === "finalOutcome")) a.push(kw);
      switch (method) {
        case "initialState":
          return enc(stateValue(ev.initialState ? ev.initialState(...a) : null));
        case "nextState": {
          const next = ev.nextState(...a);
          return enc(next === Reroll ? next : stateValue(next));
        }
        case "extraOutcomes":
          return enc(ev.extraOutcomes ? Array.from(ev.extraOutcomes(...a)) : []);
        case "finalOutcome":
          // By default the final outcome is the final state.
          return enc(ev.finalOutcome ? ev.finalOutcome(...a) : a[0]);
      }
      throw new Error(`unknown evaluator method ${method}`);
    });
  },
};

// --- Handles ------------------------------------------------------------------------------------------------------

const HANDLE = Symbol("handle"); // constructor token: wrap an existing handle
// Frees the handle of a collected object. The registry is the only release for objects nobody releases, but the
// collector frees an object when it likes (in a loop that makes many, the WASM memory grows meanwhile), and never
// frees one a JS function the core holds refers to: such a function (a `mapCounts` callback, a multiset function)
// sits in `functions` until the core drops it, and the core drops it only when the handle is released. Hence
// `release()`.
const registry = new FinalizationRegistry((h) => W && wasm(() => W.release(h)));

/** The handle of `x`, which must not have been released. */
function handleOf(x) {
  if (x._h < 0) throw new TypeError("icepool: this object was released");
  return x._h;
}

/** A core object (die, pool, deck, ...) held by JS. */
export class Handle {
  constructor(token, h) {
    if (token !== HANDLE) throw new TypeError("icepool: use the factory functions to make core objects");
    this._h = h;
    // -1: a constructor about to adopt a handle (see `adopt`).
    if (h >= 0) registry.register(this, h, this);
  }
  /** The harness's wire encoding of this object (`{"ok": true, "die" | "value": ...}`), as JSON text. */
  toWire() {
    return wasm(() => W.wire_result(handleOf(this)));
  }
  /** Frees the object's WASM memory now rather than when it is garbage-collected. Using it afterwards throws a
   * `TypeError`; releasing it again does nothing. Objects made from it (`die.add(1)`) are not affected. */
  release() {
    if (this._h < 0) return;
    const h = this._h;
    registry.unregister(this);
    this._h = -1;
    wasm(() => W.release(h));
  }
  /** `release()`, for `using die = d(6);` (runtimes without `Symbol.dispose` get Node's older name for it). */
  [Symbol.dispose ?? Symbol.for("nodejs.dispose")]() {
    this.release();
  }
}

/** Adopts the handle of a freshly made object into `self` (for `new Die(...)` and friends). */
function adopt(self, made, cls) {
  if (!(made instanceof cls)) throw new TypeError(`icepool: expected a ${cls.name}`);
  registry.unregister(made);
  self._h = made._h;
  made._h = -1;
  registry.register(self, self._h, self);
  return self;
}

/** A plain object: keyword arguments (no value the core takes is one). */
const isOptions = (x) => x !== null && typeof x === "object" && Object.getPrototypeOf(x) === Object.prototype;

/** Splits a trailing options object off a rest-parameter list. */
function opts(args) {
  return isOptions(args[args.length - 1]) ? [args.slice(0, -1), args[args.length - 1]] : [args, {}];
}

const list = (xs) => (xs instanceof List ? xs : Array.isArray(xs) ? List.from(xs) : xs);
const big = (x) => (x === null || x === undefined ? x : BigInt(x));
const nul = (x) => (x === undefined ? null : x);

/** A Map's items as a list of pairs (Python's `dict.items()` for `dict(items)`). */
const mapItems = (m) => List.from(Array.from(m, ([k, v]) => [k, v]));

// --- Operators ----------------------------------------------------------------------------------------------------

const BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor"];
const COMPARE = ["lt", "le", "gt", "ge", "eq", "ne"];
const UNARY = ["neg", "pos", "abs", "invert", "trunc", "floor", "ceil"];

/** `a <op> b` with Python's semantics, for any operands (dice, Again expressions or plain values). */
export const ops = Object.fromEntries([
  ...[...BINARY, ...COMPARE].map((op) => [op, (a, b) => call(op, { a, b })]),
  ...UNARY.map((op) => [op, (a) => call(op, { a })]),
  ["round", (a, ndigits = null) => call("round", { a, ndigits })],
  ["matmul", (n, die) => call("matmul", { n, die })],
]);

/** The operator methods shared by dice and Again expressions. `div` is `truediv`. */
class Operand extends Handle {}
for (const op of [...BINARY, ...COMPARE]) {
  Operand.prototype[op] = function (other) {
    return call(op, { a: this, b: other });
  };
}
for (const op of UNARY) {
  Operand.prototype[op] = function () {
    return call(op, { a: this });
  };
}
Operand.prototype.div = Operand.prototype.truediv;
Operand.prototype.round = function (ndigits = null) {
  return call("round", { a: this, ndigits });
};

// --- Die ----------------------------------------------------------------------------------------------------------

/** `icepool.Die`. `new Die([1, 2, 3])`, `new Die([d6, d8], [2, 1])`, `new Die(new Map([[1, 2n], [2, 1n]]))`;
 * options `{ againCount, againDepth, againEnd }` as in Python. */
export class Die extends Operand {
  constructor(outcomes, times, options) {
    if (outcomes === HANDLE) {
      super(HANDLE, times);
      return;
    }
    super(HANDLE, -1);
    if (isOptions(times)) [times, options] = [undefined, times];
    adopt(this, makeDie(outcomes, times, options ?? {}), Die);
  }

  /** `n @ die`: the sum of n rolls (n an int or a die). */
  static sum(n, die) {
    return call("matmul", { n, die });
  }

  // Selection and rerolls. `which` is a function, an array of outcomes, or null (Python's defaults).
  /** `depth` is required, as in Python: an int, or Infinity (Python's 'inf'). */
  reroll(which = null, { star = null, depth } = {}) {
    return call("reroll", { die: this, which: list(which), star, depth: depth === Infinity ? "inf" : depth });
  }
  filter(which, { star = null, depth } = {}) {
    return call("filter", { die: this, which: list(which), star, depth: depth === Infinity ? "inf" : depth });
  }
  truncate(min = null, max = null) {
    return call("truncate", { die: this, min, max });
  }
  clip(min = null, max = null) {
    return call("clip", { die: this, min, max });
  }
  /** `[selected, rest]`. */
  split(which = null, { star = null } = {}) {
    return call("split", { die: this, which: list(which), star });
  }
  explode(which = null, { star = null, depth = 9, end = null } = {}) {
    return call("explode", { die: this, which: list(which), star, depth, end });
  }

  // map and friends: `die.map(f, ...extraArgs, { star, repeat })` is `map(f, die, ...extraArgs, ...)`.
  map(f, ...rest) {
    return map(f, this, ...rest);
  }
  mapAndTime(f, ...rest) {
    return mapAndTime(f, this, ...rest);
  }
  meanTimeToAbsorb(f, ...rest) {
    return meanTimeToAbsorb(f, this, ...rest);
  }
  /** The expected number of rolls to reach a sum of at least `target` (a Fraction). */
  meanTimeToSum(target) {
    return call("mean_time_to_sum", { die: this, targets: List.of(target) })[0];
  }
  timeToSum(target, maxTime = null, dnf = null) {
    return call("time_to_sum", { die: this, target, max_time: maxTime, dnf });
  }
  groupBy(key, { star = null } = {}) {
    return new Map(call("group_by", { die: this, key, star }));
  }
  groupByIndex(index) {
    return new Map(call("group_by_index", { die: this, index }));
  }
  isIn(outcomes) {
    return call("is_in", { die: this, outcomes: list(outcomes) });
  }
  count(rolls, outcomes) {
    return call("die_count", { die: this, rolls, outcomes: list(outcomes) });
  }
  ifElse(ifTrue, ifFalse, { againCount = null, againDepth = null, againEnd = null } = {}) {
    return call("if_else", {
      die: this,
      if_true: ifTrue,
      if_false: ifFalse,
      count: againCount,
      depth: againDepth,
      end: againEnd,
    });
  }

  // Rolling several.
  highest(rolls, keep = null, drop = null) {
    return call("die_highest", { die: this, rolls, keep, drop });
  }
  lowest(rolls, keep = null, drop = null) {
    return call("die_lowest", { die: this, rolls, keep, drop });
  }
  middle(rolls, keep = 1, { tie = "error" } = {}) {
    return call("die_middle", { die: this, rolls, keep, tie });
  }
  /** `die.keep(rolls, index)`: roll `rolls` dice, sort them and sum the selected ones. `rolls` an array of counts
   * (one per sorted roll) and no index; or an int and an index: an int (the roll at that sorted position) or an
   * array of counts. */
  keep(rolls, index = null) {
    return call("die_keep", { die: this, rolls, index });
  }
  /** `die.keep(rolls, slice(start, stop, step))`: the sorted rolls in the slice, summed. */
  keepSlice(rolls, start = null, stop = null, step = null) {
    return call("die_keep_slice", { die: this, rolls, start, stop, step });
  }
  /** Every ordered sequence of `rolls` rolls, as a die of arrays (Python's tuples); `[]` itself for 0 rolls. */
  sequence(rolls) {
    return call("die_sequence", { die: this, rolls });
  }
  /** `rolls` copies of this die as a pool (a list of counts: a keep tuple). */
  pool(rolls = 1) {
    return call("die_pool", { die: this, rolls: Array.isArray(rolls) ? list(rolls) : rolls });
  }

  // Pools from a die (EXPERIMENTAL in icepool).
  /** `die.map_to_pool(f, ...extraArgs, { star })`: `mapToPool(f, die, ...extraArgs)`; `f` null uses the outcomes
   * themselves as the pools' contents. */
  mapToPool(f = null, ...rest) {
    const [args, { star = null }] = opts(rest);
    return call("die_map_to_pool", { die: this, f, args: List.from(args), star });
  }
  /** Each explosion is a separate die of the pool; `outcomes` null explodes the max outcome. */
  explodeToPool(rolls = 1, outcomes = null, { star = null, depth = 9 } = {}) {
    return call("explode_to_pool", { die: this, rolls, outcomes: list(outcomes), star, depth });
  }
  /** At most `maxRerolls` rerolls (an int or Infinity) shared across a pool of `rolls` dice; `mode` is 'random',
   * 'low', 'high' or 'drop'. */
  rerollToPool(rolls, outcomes = null, { maxRerolls, star = null, depth = 1, mode = "random" } = {}) {
    return call("reroll_to_pool", {
      die: this,
      rolls,
      outcomes: list(outcomes),
      max_rerolls: maxRerolls === Infinity ? "inf" : maxRerolls,
      star,
      depth: depth === Infinity ? "inf" : depth,
      mode,
    });
  }

  // Transformations.
  simplify() {
    return call("simplify", { die: this });
  }
  zero() {
    return call("zero", { die: this });
  }
  zeroOutcome() {
    return call("zero_outcome", { die: this });
  }
  sign() {
    return call("sign", { die: this });
  }
  cmp(other) {
    return call("die_cmp", { die: this, other });
  }
  padToDenominator(denominator, outcome) {
    return call("pad_to_denominator", { die: this, denominator, outcome });
  }
  multiplyToDenominator(denominator) {
    return call("multiply_to_denominator", { die: this, denominator });
  }
  multiplyQuantities(scale) {
    return call("quantities_op", { die: this, method: "multiply_quantities", x: scale });
  }
  divideQuantities(divisor) {
    return call("quantities_op", { die: this, method: "divide_quantities", x: divisor });
  }
  moduloQuantities(divisor) {
    return call("quantities_op", { die: this, method: "modulo_quantities", x: divisor });
  }
  /** Each outcome rounded up with probability `x - floor(x)`, else down. */
  stochasticRound({ maxDenominator = null } = {}) {
    return call("die_stochastic_round", { die: this, max_denominator: maxDenominator });
  }
  equals(other, { simplify = false } = {}) {
    return call("equals", { a: this, b: other, simplify });
  }

  // The mapping protocol: outcomes -> quantities (BigInts).
  items() {
    return call("die_method", { die: this, name: "items", arg: null }).map(([o, q]) => [o, BigInt(q)]);
  }
  outcomes() {
    return call("die_method", { die: this, name: "keys", arg: null });
  }
  keys() {
    return this.outcomes();
  }
  values() {
    return call("die_method", { die: this, name: "values", arg: null }).map(BigInt);
  }
  /** The quantity of an outcome (KeyError if it is not one). */
  get(outcome) {
    return BigInt(call("die_method", { die: this, name: "getitem", arg: outcome }));
  }
  has(outcome) {
    return call("die_method", { die: this, name: "contains", arg: outcome });
  }
  get size() {
    return call("die_method", { die: this, name: "len", arg: null });
  }
  [Symbol.iterator]() {
    return this.outcomes()[Symbol.iterator]();
  }

  // Statistics. Exact ones are Fractions, the rest floats (numbers).
  mean() { return dieStat(this, "mean"); }
  variance() { return dieStat(this, "variance"); }
  standardDeviation() { return dieStat(this, "standard_deviation"); }
  sd() { return dieStat(this, "sd"); }
  skewness() { return dieStat(this, "skewness"); }
  excessKurtosis() { return dieStat(this, "excess_kurtosis"); }
  mode() { return dieStat(this, "mode"); }
  modalQuantity() { return big(dieStat(this, "modal_quantity")); }
  median() { return dieStat(this, "median"); }
  medianLow() { return dieStat(this, "median_low"); }
  medianHigh() { return dieStat(this, "median_high"); }
  minOutcome() { return dieStat(this, "min_outcome"); }
  maxOutcome() { return dieStat(this, "max_outcome"); }
  denominator() { return big(dieStat(this, "denominator")); }
  isEmpty() { return dieStat(this, "is_empty"); }
  quantile(n, d = 100) { return call("die_quantile", { die: this, method: "quantile", args: List.of(n, d) }); }
  quantileLow(n, d = 100) { return call("die_quantile", { die: this, method: "quantile_low", args: List.of(n, d) }); }
  quantileHigh(n, d = 100) { return call("die_quantile", { die: this, method: "quantile_high", args: List.of(n, d) }); }
  standardizedMoment(k) { return call("standardized_moment", { die: this, k }); }
  entropy(base = 2.0) { return call("entropy", { die: this, base: List.of(base) }); }
  kolmogorovSmirnov(other) { return call("cdf_distance", { a: this, b: other, stat: "kolmogorov_smirnov" }); }
  cramerVonMises(other) { return call("cdf_distance", { a: this, b: other, stat: "cramer_von_mises" }); }
  covariance(i, j) { return call("joint_stat", { die: this, stat: "covariance", i, j }); }
  correlation(i, j) { return call("joint_stat", { die: this, stat: "correlation", i, j }); }
  nearest(comparison, outcome) { return call("nearest", { die: this, comparison, outcome }); }
  commonOutcomeLength() { return call("common_outcome_length", { die: this }); }

  /** `quantity(outcome)` or `quantity(comparison, outcome)`. */
  quantity(...args) {
    return BigInt(call("quantity", { die: this, args: List.from(args) }));
  }
  /** All quantities, or cumulative ones for a comparison ('<=', '>=', ...). */
  quantities(comparison = null) {
    return call("quantities", { die: this, comparison }).map(BigInt);
  }
  probability(...args) {
    const [a, { percent = false }] = opts(args);
    return call("probability", { die: this, args: List.from(a), percent });
  }
  probabilities(comparison = null, { percent = false } = {}) {
    return call("probabilities", { die: this, comparison, percent });
  }
  quantityWhere(which, { star = null } = {}) {
    return BigInt(call("quantity_where", { die: this, which, star }));
  }
  probabilityWhere(which, { star = null, percent = false } = {}) {
    return call("probability_where", { die: this, which, star, percent });
  }

  // Tuple outcomes.
  marginals() {
    return call("marginals", { die: this });
  }
  marginal(index) {
    return call("marginal", { die: this, index });
  }
  marginalSlice(start = null, stop = null, step = null) {
    return call("marginal_slice", { die: this, start, stop, step });
  }

  /** One outcome at random (Python's Mersenne Twister seeded with `seed`), or `n` of them as an array. */
  sample({ seed = Math.floor(Math.random() * 2 ** 32), n } = {}) {
    const rolls = call("sample", { die: this, seed, n: n ?? 1 });
    return n === undefined ? rolls[0] : rolls;
  }

  /** `format(die, spec)`: a Markdown / BBCode / CSV / HTML table (`spec` as Python's, e.g. "md:q==|p"). */
  format(spec = null) {
    return call("format", { die: this, spec });
  }
  toString() {
    return this.format(null);
  }
  /** Python's `repr(die)`. */
  repr() {
    return call("die_repr", { die: this });
  }
}

function dieStat(die, stat) {
  return call("die_stat", { die, stat });
}

function makeDie(outcomes, times, { againCount, againDepth, againEnd }) {
  const again = againCount !== undefined || againDepth !== undefined || againEnd !== undefined;
  if (outcomes instanceof Map) {
    if (times !== undefined || again) throw new TypeError("icepool: a Map of quantities takes no times or again");
    return call("weights", { items: mapItems(outcomes) });
  }
  outcomes = list(outcomes);
  if (again) {
    if (times !== undefined) throw new IcepoolError("NotImplementedError", "times together with again options");
    if (againCount !== undefined && againCount !== null) {
      return call("again_count", { outcomes, count: againCount, depth: nul(againDepth), end: nul(againEnd) });
    }
    return call("again", { outcomes, depth: nul(againDepth), end: nul(againEnd) });
  }
  if (times !== undefined) return call("die_times", { outcomes, times: list(times) });
  return call("die", { outcomes });
}

/** The result of `die.eq(x)` / `die.ne(x)`: a die of bools that also has a truth value (`equals`). */
export class DieWithTruth extends Die {
  constructor(token, h, truth) {
    super(token, h);
    this._truth = truth;
  }
  /** Python's `bool(die == x)`. */
  truth() {
    return this._truth;
  }
}

/** `icepool.Die(outcomes, times, options)` without `new`. */
export function die(outcomes, times, options) {
  return new Die(outcomes, times, options);
}

/** A fair die numbered 1..sides. */
export function d(sides) {
  return call("d", { sides });
}
/** A fair die numbered 0..sides-1. */
export function z(sides) {
  return call("z", { sides });
}
export function coin(n, d = 1, { maxDenominator = null } = {}) {
  return call("coin", { n, d, max_denominator: maxDenominator });
}
export function oneHot(sides) {
  return call("one_hot", { sides });
}
export function stochasticRound(x, { maxDenominator = null } = {}) {
  return call("stochastic_round", { x, max_denominator: maxDenominator });
}
export function fromCumulative(outcomes, cumulative, { reverse = false } = {}) {
  return call("from_cumulative", { outcomes: list(outcomes), cumulative: list(cumulative), reverse });
}

/** `icepool.from_rv(rv, outcomes, denominator, **kwargs)`: a die from a random variable's CDF, as Python's, which
 * duck-types `scipy.stats` objects: `rv.cdf(xs, kwargs)` takes an array (a `List`) and returns an iterable of
 * probabilities, called once; an rv with a `pdf` property is continuous (the CDF is taken at the midpoints between
 * the outcomes). `kwargs` reach `cdf` as a trailing options object, when there are any. */
export function fromRv(rv, outcomes, denominator, kwargs = {}) {
  if (rv === null || typeof rv !== "object" || typeof rv.cdf !== "function") {
    throw new TypeError("icepool: fromRv takes an object with a cdf method");
  }
  // `hasattr(rv, 'pdf')`.
  const pdf = "pdf" in rv;
  const cdf = (...args) => rv.cdf(...args);
  const args = `{"cdf":${enc(cdf)},"pdf":${pdf},"outcomes":${enc(outcomes)},"denominator":${enc(denominator)},"kwargs":${argsText(kwargs)}}`;
  return dec(unwrap(wasm(() => W.from_rv(args))));
}

// --- Free functions -----------------------------------------------------------------------------------------------

/** `icepool.map(f, ...args, { star, repeat, ...kwargs })`: `repeat` an int or Infinity; any other option is passed
 * to `f` as a keyword argument (a trailing options object in JS). */
export function map(f, ...rest) {
  const [args, { star = null, repeat, ...kwargs }] = opts(rest);
  const a = List.from(args);
  if (repeat !== undefined) {
    return call("map_repeat", { f, args: a, star, repeat: repeat === Infinity ? "inf" : repeat });
  }
  if (Object.keys(kwargs).length) {
    return call("map_kw", { f, args: a, star, kwargs: List.from(Object.entries(kwargs)) });
  }
  return call("map", { f, args: a, star });
}

/** `icepool.map_and_time(f, initial, ...args, { star, repeat })`: a die of [state, time to absorb]. */
export function mapAndTime(f, initial, ...rest) {
  const [args, { star = null, repeat = "inf" }] = opts(rest);
  return call("map_and_time", {
    f,
    initial,
    args: List.from(args),
    star,
    repeat: repeat === Infinity ? "inf" : repeat,
  });
}

export function meanTimeToAbsorb(f, initial, ...rest) {
  const [args, { star = null }] = opts(rest);
  return call("mean_time_to_absorb", { f, initial, args: List.from(args), star });
}

/** `icepool.map_to_pool(f, ...args, { star })` (EXPERIMENTAL in icepool): `f`'s result for each joint outcome of
 * the args (dice expand, anything else as it is) is a pool (what `new Pool(...)` takes, a `List` of dice or
 * outcomes, or a multiset expression; `Reroll` drops the outcome); the result is the mixture of those pools. */
export function mapToPool(f, ...rest) {
  const [args, { star = null }] = opts(rest);
  return call("map_to_pool", { f, args: List.from(args), star });
}

/** `icepool.map_function(f, { star, ...kwargs })`: `f` as a function of dice (decorator form: `star` is kept). */
export function mapFunction(f, { star = null, ...kwargs } = {}) {
  return (...args) =>
    call("map_function", {
      f,
      args: List.from(args),
      star,
      kwargs: List.from(Object.entries(kwargs)),
      decorator: true,
    });
}

export function reduce(f, dice, { initial = null } = {}) {
  return call("reduce", { f, dice: list(dice), initial });
}
export function accumulate(f, dice, { initial = null } = {}) {
  return call("accumulate", { f, dice: list(dice), initial });
}

function keepFunction(op) {
  return (...rest) => {
    const [args, o] = opts(rest);
    const extra = op === "middle" ? { keep: o.keep ?? 1, tie: o.tie ?? "error" } : { keep: nul(o.keep), drop: nul(o.drop) };
    return call(op, { args: List.from(args), ...extra, default: nul(o.default) });
  };
}
/** `highest(...dice, { keep, drop, default })`: the sum of the highest rolls among the dice. */
export const highest = keepFunction("highest");
export const lowest = keepFunction("lowest");
/** `middle(...dice, { keep, tie, default })`. */
export const middle = keepFunction("middle");

export function tupleize(...args) {
  return call("tupleize", { args: List.from(args) });
}
export function vectorize(...args) {
  return call("vectorize", { args: List.from(args) });
}
export function pointwiseMax(...dice) {
  return call("pointwise", { args: List.from(dice), max: true, spread: true });
}
export function pointwiseMin(...dice) {
  return call("pointwise", { args: List.from(dice), max: false, spread: true });
}
export function maxOutcome(...args) {
  return call("extreme_outcome", { args: List.from(args), max: true, spread: true });
}
export function minOutcome(...args) {
  return call("extreme_outcome", { args: List.from(args), max: false, spread: true });
}
export function consecutive(...args) {
  return call("consecutive", { args: List.from(args.map(list)) });
}
export function sortedUnion(...args) {
  return call("sorted_union", { args: List.from(args.map(list)) });
}
export function harmonizeDenominators(dice, weights = null) {
  return call("harmonize_denominators", { dice: list(dice), weights: list(weights) });
}
export function formatProbabilityInverse(probability, intStart = 20) {
  return call("format_probability_inverse", { probability, args: List.of(intStart) });
}

// --- Pools and multiset expressions -------------------------------------------------------------------------------

/** Any `MultisetExpression`: a pool, a deal, or an operator over them. Evaluation methods return dice. */
export class MultisetExpression extends Handle {
  // Multiset operators: `+` `-` `&` `|` `^` (the other side may be an array of outcomes).
  additiveUnion(...others) {
    return mxMethod("additive_union", [this, ...others]);
  }
  difference(...rest) {
    const [others, { keepNegativeCounts = false }] = opts(rest);
    return call("mx_method", { name: "difference", args: List.of(this, ...others.map(list)), keep_negative_counts: keepNegativeCounts });
  }
  intersection(...others) {
    return mxMethod("intersection", [this, ...others]);
  }
  union(...others) {
    return mxMethod("union", [this, ...others]);
  }
  symmetricDifference(other) {
    return call("mx_binary", { a: this, o: "^", b: list(other) });
  }
  add(other) {
    return call("mx_binary", { a: this, o: "+", b: list(other) });
  }
  sub(other) {
    return call("mx_binary", { a: this, o: "-", b: list(other) });
  }
  and(other) {
    return call("mx_binary", { a: this, o: "&", b: list(other) });
  }
  or(other) {
    return call("mx_binary", { a: this, o: "|", b: list(other) });
  }
  xor(other) {
    return call("mx_binary", { a: this, o: "^", b: list(other) });
  }

  // Counts.
  mul(n) { return mxCounts(this, "mul", null, n); }
  neg() { return mxCounts(this, "neg", null, null); }
  pos() { return mxCounts(this, "pos", null, null); }
  floordiv(n) { return mxCounts(this, "floordiv", null, n); }
  mod(n) { return mxCounts(this, "mod", null, n); }
  unique(n = 1) { return mxCounts(this, "unique", null, n); }
  keepCounts(comparison, n) { return mxCounts(this, "keep_counts", comparison, n); }
  multiplyCounts(n) { return mxCounts(this, "multiply_counts", null, n); }
  divideCounts(n) { return mxCounts(this, "divide_counts", null, n); }
  moduloCounts(n) { return mxCounts(this, "modulo_counts", null, n); }

  // Outcomes.
  keepOutcomes(target) { return call("mx_filter", { expr: this, invert: false, target: list(target) }); }
  dropOutcomes(target) { return call("mx_filter", { expr: this, invert: true, target: list(target) }); }
  forceOrder(order) { return call("mx_force_order", { expr: this, order }); }
  // `MultisetGenerator.weightless()`: each possible multiset weighted 1. Only generators (pools, deals, compound
  // keeps, weightless ones) have it in Python; on another expression the core raises AttributeError.
  weightless() { return call("weightless", { generator: this }); }

  // Pairing.
  maxPairKeep(comparison, other, priority = null) {
    return call("mx_max_pair", { expr: this, keep: true, comparison, other: list(other), priority });
  }
  maxPairDrop(comparison, other, priority = null) {
    return call("mx_max_pair", { expr: this, keep: false, comparison, other: list(other), priority });
  }
  sortPair(comparison, other, order = Order.Descending, extra = "drop") {
    return sortPair(this, "sort_pair", comparison, other, order, extra);
  }
  sortPairKeepWhile(comparison, other, order = Order.Descending, extra = "drop") {
    return sortPair(this, "sort_pair_keep_while", comparison, other, order, extra);
  }
  sortPairDropWhile(comparison, other, order = Order.Descending, extra = "drop") {
    return sortPair(this, "sort_pair_drop_while", comparison, other, order, extra);
  }
  versusAll(comparison, other) {
    return call("mx_versus", { expr: this, all: true, comparison, other: list(other) });
  }
  versusAny(comparison, other) {
    return call("mx_versus", { expr: this, all: false, comparison, other: list(other) });
  }

  // Evaluation.
  sum() { return mxEvaluate(this, "sum"); }
  size() { return mxEvaluate(this, "size"); }
  empty() { return mxEvaluate(this, "empty"); }
  any() { return mxEvaluate(this, "any"); }
  productOfCounts() { return mxEvaluate(this, "product_of_counts"); }
  highestOutcomeAndCount() { return mxEvaluate(this, "highest_outcome_and_count"); }
  largestCount() { return mxEvaluate(this, "largest_count"); }
  largestCountAndOutcome() { return mxEvaluate(this, "largest_count_and_outcome"); }
  largestStraight() { return mxEvaluate(this, "largest_straight"); }
  allStraights() { return mxEvaluate(this, "all_straights"); }
  largestStraightAndOutcome(priority = "high") {
    return call("pool_largest_straight_and_outcome", { pool: this, priority });
  }
  /** All elements as a sorted tuple (a die of arrays). */
  expand(order = Order.Ascending) {
    return call("pool_expand", { pool: this, order });
  }
  allCounts(filter = 1) {
    return call("pool_all_counts", { pool: this, filter });
  }
  countSubset(divisor, { emptyDivisor = null } = {}) {
    return call("pool_count_subset", { pool: this, divisor: list(divisor), empty_divisor: emptyDivisor });
  }
  isdisjoint(other) { return call("pool_isdisjoint", { pool: this, other: list(other) }); }
  issubset(other) { return call("pool_issubset", { pool: this, other: list(other) }); }
  issuperset(other) { return call("pool_issuperset", { pool: this, other: list(other) }); }
  /** `self < other` (a proper subset), as a die of bools. */
  lt(other) { return call("pool_compare", { pool: this, other: list(other), cmp: "proper_subset" }); }
  gt(other) { return call("pool_compare", { pool: this, other: list(other), cmp: "proper_superset" }); }
  /** `self == other` as multisets (a die of bools). */
  equalSet(other) { return call("pool_compare", { pool: this, other: list(other), cmp: "equal_set" }); }
  notEqualSet(other) { return call("pool_compare", { pool: this, other: list(other), cmp: "not_equal_set" }); }

  /** `MultisetExpression.map_counts(...args, { function })`. */
  static mapCounts(...rest) {
    const [args, { function: f }] = opts(rest);
    return call("mx_map_counts", { args: List.from(args.map(list)), function: f });
  }
}

function mxMethod(name, args) {
  return call("mx_method", { name, args: List.from(args.map(list)), keep_negative_counts: false });
}
function mxCounts(expr, o, comparison, n) {
  return call("mx_counts", { expr, o, comparison, n });
}
function mxEvaluate(expr, method) {
  return call("mx_evaluate", { expr, method });
}
function sortPair(expr, method, comparison, other, order, extra) {
  return call("mx_sort_pair", { expr, method, comparison, other: list(other), order, extra });
}

/** A `KeepGenerator`: a pool or a deal, whose sorted rolls can be kept or dropped. */
class KeepGenerator extends MultisetExpression {
  highest(keep = null, drop = null) {
    return call("pool_highest", { pool: this, keep, drop });
  }
  lowest(keep = null, drop = null) {
    return call("pool_lowest", { pool: this, keep, drop });
  }
  middle(keep = 1, { tie = "error" } = {}) {
    return call("pool_middle", { pool: this, keep, tie });
  }
  /** `pool.keep(index)`: an array of counts (a keep tuple, `Ellipsis` allowed) or an int (the kept roll as a die).
   * An array is a tuple, as everywhere: a `List` index is unhashable (a mixture of such keeps raises TypeError). */
  keep(index) {
    return call("pool_keep", { pool: this, index });
  }
  /** `pool[index]`: the roll at that sorted position, as a die. */
  at(index) {
    return call("pool_index", { pool: this, index });
  }
  /** `pool[start:stop:step]`. */
  slice(start = null, stop = null, step = null) {
    return call("pool_slice", { pool: this, start, stop, step });
  }
}

/** `icepool.Pool`: `new Pool([d6, d6, d8])`, `new Pool([d6, d8], [2, 1])`, `new Pool(new Map([[d6, 3]]))`. */
export class Pool extends KeepGenerator {
  constructor(dice, times = 1) {
    if (dice === HANDLE) {
      super(HANDLE, times);
      return;
    }
    super(HANDLE, -1);
    const made =
      dice instanceof Map
        ? call("pool_dict", { items: mapItems(dice), times: list(times) })
        : call("pool", { dice: list(dice), times: list(times) });
    adopt(this, made, Pool);
  }
  /** `(keep_tuple(), keep_size(), raw_size(), denominator(), outcomes(), _dice)` */
  info() {
    return call("pool_info", { pool: this });
  }
  keepTuple() { return this.info()[0]; }
  keepSize() { return this.info()[1]; }
  rawSize() { return this.info()[2]; }
  denominator() { return BigInt(this.info()[3]); }
  outcomes() { return this.info()[4]; }
}

export function pool(dice, times = 1) {
  return new Pool(dice, times);
}
/** One d(size) per size; a Map of size -> count. */
export function dPool(dieSizes) {
  return dieSizes instanceof Map
    ? call("d_pool", { die_sizes: mapItems(dieSizes), mapping: true })
    : call("d_pool", { die_sizes: list(dieSizes), mapping: false });
}
export function zPool(dieSizes) {
  return dieSizes instanceof Map
    ? call("z_pool", { die_sizes: mapItems(dieSizes), mapping: true })
    : call("z_pool", { die_sizes: list(dieSizes), mapping: false });
}

/** `icepool.MultisetMixture(inners)`. */
export function multisetMixture(inners) {
  return call("mx_mixture", { inners: list(inners) });
}

/** Any other multiset expression (an operator over pools, a CompoundKeepGenerator...). */
export class Expression extends MultisetExpression {}

// --- Decks and deals ----------------------------------------------------------------------------------------------

/** `icepool.Deck`: `new Deck([1, 2, 3], 4)` (each card 4 times), `new Deck(new Map([["a", 2]]))`. */
export class Deck extends Handle {
  constructor(outcomes, times = 1) {
    if (outcomes === HANDLE) {
      super(HANDLE, times);
      return;
    }
    super(HANDLE, -1);
    const made =
      outcomes instanceof Map
        ? call("deck", { outcomes: List.from(outcomes.keys()), times: List.from(outcomes.values()) })
        : call("deck", { outcomes: list(outcomes), times: list(times) });
    adopt(this, made, Deck);
  }
  /** A Deal of `handSize` cards, or a MultiDeal for an array of hand sizes. */
  deal(handSize) {
    return Array.isArray(handSize)
      ? call("multi_deal", { deck: this, hand_sizes: list(handSize) })
      : call("deal", { deck: this, hand_size: handSize });
  }
  /** `deck.deal_groups(...[handSize, groupSize])`. */
  dealGroups(...groups) {
    return call("deal_groups", { deck: this, hand_groups: List.from(groups.map(list)) });
  }
  map(f, { star = null } = {}) {
    return call("deck_map", { deck: this, repl: f, star });
  }
  add(other) { return call("deck_op", { operator: "+", a: this, b: list(other) }); }
  sub(other) { return call("deck_op", { operator: "-", a: this, b: list(other) }); }
  and(other) { return call("deck_op", { operator: "&", a: this, b: list(other) }); }
  or(other) { return call("deck_op", { operator: "|", a: this, b: list(other) }); }
  xor(other) { return call("deck_op", { operator: "^", a: this, b: list(other) }); }
  mul(n) { return call("deck_mul", { deck: this, operator: "*", n }); }
  floordiv(n) { return call("deck_mul", { deck: this, operator: "//", n }); }
  mod(n) { return call("deck_mul", { deck: this, operator: "%", n }); }
  symmetricDifference(other) {
    return call("deck_method", { deck: this, name: "symmetric_difference", arg: list(other) });
  }
  padToDenominator(denominator, outcome) {
    return call("deck_pad_to_denominator", { deck: this, denominator, outcome });
  }
  /** Every sequence of `deals` cards dealt one by one, as a die of arrays. */
  sequence(deals) {
    return call("deck_sequence", { deck: this, deals });
  }
  items() {
    return call("deck_info", { deck: this })[0].map(([o, q]) => [o, BigInt(q)]);
  }
  outcomes() {
    return call("deck_method", { deck: this, name: "keys", arg: null });
  }
  keys() {
    return this.outcomes();
  }
  values() {
    return call("deck_method", { deck: this, name: "values", arg: null }).map(BigInt);
  }
  get(outcome) {
    return BigInt(call("deck_method", { deck: this, name: "getitem", arg: outcome }));
  }
  /** The number of cards (`deck.size()`). */
  size() {
    return BigInt(call("deck_info", { deck: this })[1]);
  }
  get length() {
    return call("deck_info", { deck: this })[2];
  }
  [Symbol.iterator]() {
    return this.outcomes()[Symbol.iterator]();
  }
  // Statistics as of a die with these quantities.
  stat(name) {
    return call("deck_stat", { deck: this, stat: name });
  }
  mean() { return this.stat("mean"); }
  variance() { return this.stat("variance"); }
  sd() { return this.stat("sd"); }
  minOutcome() { return this.stat("min_outcome"); }
  maxOutcome() { return this.stat("max_outcome"); }
  denominator() { return BigInt(this.stat("denominator")); }
}

export function deck(outcomes, times = 1) {
  return new Deck(outcomes, times);
}

/** `deck.deal(handSize)`: a multiset expression over the hand. */
export class Deal extends KeepGenerator {
  /** `(deck items, keep_tuple, hand_size, keep_size, denominator, outcomes)` */
  info() {
    return call("deal_info", { deal: this });
  }
  handSize() { return this.info()[2]; }
  keepSize() { return this.info()[3]; }
  denominator() { return BigInt(this.info()[4]); }
  outcomes() { return this.info()[5]; }
  toString() { return call("deal_str", { deal: this })[1]; }
  repr() { return call("deal_str", { deal: this })[0]; }
}

/** A multiset tuple expression: several hands (a `MultiDeal`, a tuple parameter of a multiset function, a slice
 * of either). `hand(i)` is one hand (Python's `x[i]`), `slice(...)` some of them. */
export class MultisetTupleExpression extends Handle {
  hand(index) {
    return call("mx_tuple_index", { expr: this, index });
  }
  slice(start = null, stop = null, step = null) {
    return call("mx_tuple_slice", { expr: this, start, stop, step });
  }
}

/** `deck.deal([size, ...])`: several hands; `hand(i)` or `slice(...)` picks some. */
export class MultiDeal extends MultisetTupleExpression {
  /** `(deck items, hand_sizes, total_cards_dealt, len, denominator, outcomes)` */
  info() {
    return call("multi_deal_info", { multi_deal: this });
  }
  handSizes() { return this.info()[1]; }
  totalCardsDealt() { return this.info()[2]; }
  denominator() { return BigInt(this.info()[4]); }
  outcomes() { return this.info()[5]; }
  toString() { return call("deal_str", { deal: this })[1]; }
  repr() { return call("deal_str", { deal: this })[0]; }
}

// --- Wallenius ----------------------------------------------------------------------------------------------------

/** `icepool.Wallenius(data)` (EXPERIMENTAL in icepool): Wallenius' noncentral hypergeometric distribution, sampling
 * without replacement with weights, where the whole weight of a card goes away when it is pulled. `data` is an
 * array of `[outcome, weight]` pairs, or a Map from outcomes to a weight or a `[weight, quantity]` pair (a Python
 * dict: keys equal by Python's `==`). */
export class Wallenius {
  constructor(data) {
    this._mapping = data instanceof Map;
    this._data = this._mapping ? mapItems(data) : list(data);
    // Python checks the data in the constructor: a hand of 0 builds the weights and stops there.
    call("wallenius_deal_expr", { data: this._data, mapping: this._mapping, hand_size: 0 });
  }
  /** The multiset of `handSize` outcomes dealt, as a multiset expression (`.sum()`, `.expand()`, ...). */
  deal(handSize) {
    return call("wallenius_deal_expr", { data: this._data, mapping: this._mapping, hand_size: handSize });
  }
}

// --- Again --------------------------------------------------------------------------------------------------------

/** An `AgainExpression`: `Again`, or operators applied to it, as outcomes of `new Die(..., { againDepth })`. */
export class AgainExpression extends Operand {}

// --- Evaluators ---------------------------------------------------------------------------------------------------

/** A multiset evaluator written in JS, as a subclass of icepool's `MultisetEvaluator`:
 *
 *   nextState(state, order, outcome, ...counts)      required; return the next state, or Reroll
 *   initialState(order, outcomes, ...sizes)          optional; default null
 *   finalOutcome(finalState, order, outcomes, ...sizes)  optional; default the final state; may return a die
 *   extraOutcomes(outcomes)                          optional; extra outcomes for nextState to see
 *
 * States must be values the core can hash: numbers, BigInts, strings, booleans, null, arrays of those, or plain
 * objects, which are keyed by their JSON (so two objects with the same JSON are the same state). Arrays come back
 * as arrays, objects as fresh objects. Throw `UnsupportedOrder` to ask for the other order.
 *
 * `evaluator.evaluate(...inputs)` takes pools, deals, expressions or arrays of outcomes, and returns a die. */
export class MultisetEvaluator {
  /** `evaluate(...inputs, { ...kwargs })`: keyword arguments reach initialState and finalOutcome. */
  evaluate(...rest) {
    const [inputs, kwargs] = opts(rest);
    const args = `[${inputs.map((x) => enc(list(x))).join(",")}]`;
    const kw = argsText(kwargs);
    // The core releases the id (HOST.release) once it no longer needs the evaluator: when this call returns, or,
    // inside a multiset function's body, when the function holding the recorded evaluation is dropped.
    const id = nextId++;
    evaluators.set(id, this);
    return dec(unwrap(wasm(() => W.evaluate(id, args, kw))));
  }
}

/** A `MultisetEvaluator` from an object of methods: `evaluator({ nextState: (s, order, o, c) => ... })`. */
export function evaluator(methods) {
  return Object.assign(new MultisetEvaluator(), methods);
}

// --- Vector and Symbols -------------------------------------------------------------------------------------------

const COLL_BINARY = [...BINARY, ...COMPARE];

/** `icepool.Vector`: elementwise arithmetic; comparisons give vectors with a truth value (`truth(v)`). */
export class Vector {
  constructor(items) {
    this.items = Array.from(items);
    this.hasTruth = false;
    this.truthValue = null;
  }
  /** A vector from the core: `items` null for a `VectorWithTruthOnly` (a comparison of different lengths). */
  static _make(items, hasTruth, truthValue) {
    const v = new Vector([]);
    Object.assign(v, { items, hasTruth, truthValue });
    return v;
  }
  get length() {
    return this.items.length;
  }
  [Symbol.iterator]() {
    return this.items[Symbol.iterator]();
  }
  append(x) {
    return call("vector_method", { v: this, name: "append", arg: x });
  }
  concatenate(other) {
    return call("vector_method", { v: this, name: "concatenate", arg: other });
  }
  toString() {
    return call("vector_method", { v: this, name: "str", arg: null });
  }
  repr() {
    return call("vector_method", { v: this, name: "repr", arg: null });
  }
}

/** `icepool.Symbols`: a multiset of characters, from a string such as "aab-c". */
export class Symbols {
  constructor(str) {
    this.str = String(str);
    this._items = null; // the dict's [char, count] items, in its order, once the core made it
  }
  static _make(items, str) {
    const s = new Symbols("");
    Object.assign(s, { _items: items, str });
    return s;
  }
  static from(x) {
    return call("symbols", { x: x instanceof Map ? new RawMapping(x) : x });
  }
  method(name, arg = null) {
    return call("symbols_method", { s: this, name, arg });
  }
  issubset(other) { return this.method("issubset", other); }
  issuperset(other) { return this.method("issuperset", other); }
  isdisjoint(other) { return this.method("isdisjoint", other); }
  countSubset(divisor, { emptyDivisor = null } = {}) { return this.method("count_subset", [divisor, emptyDivisor]); }
  hasNegativeCounts() { return this.method("has_negative_counts"); }
  size() { return this.method("size"); }
  multiplyCounts(n) { return this.method("multiply_counts", n); }
  divideCounts(n) { return this.method("divide_counts", n); }
  moduloCounts(n) { return this.method("modulo_counts", n); }
  additiveUnion(...others) { return this.method("additive_union", others); }
  difference(...others) { return this.method("difference", others); }
  intersection(...others) { return this.method("intersection", others); }
  union(...others) { return this.method("union", others); }
  symmetricDifference(other) { return this.method("symmetric_difference", other); }
  /** The count of a character. */
  get(c) { return this.method("getitem", c); }
  keys() { return this.method("keys"); }
  values() { return this.method("values"); }
  toString() { return this.str; }
  repr() { return this.method("repr"); }
}

for (const op of COLL_BINARY) {
  for (const cls of [Vector, Symbols]) {
    cls.prototype[op] = function (other) {
      return call("coll_binary", { a: this, b: other, name: op });
    };
  }
}
for (const op of ["neg", "pos", "invert", "abs", "trunc", "floor", "ceil"]) {
  for (const cls of [Vector, Symbols]) {
    cls.prototype[op] = function () {
      return call("coll_unary", { a: this, name: op, ndigits: null });
    };
  }
}
Vector.prototype.div = Vector.prototype.truediv;
Vector.prototype.round = function (ndigits = null) {
  return call("coll_unary", { a: this, name: "round", ndigits });
};
/** Python's `bool(x)` of a Vector (from a comparison), Symbols, or an `Again` expression (from `eq` / `ne`). */
export function truth(x) {
  return x instanceof Vector || x instanceof Symbols ? call("coll_truth", { x }) : call("truth", { x });
}

// A Map passed to Symbols.from: Python's dict, as the program language spells one ({"symbols": {...}} literal).
class RawMapping extends RawJSON {
  constructor(m) {
    super(`{"symbols":{${Array.from(m, ([k, v]) => `${JSON.stringify(String(k))}:${enc(v)}`).join(",")}}}`);
  }
}

// --- Singletons and kinds -----------------------------------------------------------------------------------------

/** A callable the core made (the harness's catalogue functions). */
export class CoreFunction extends Handle {}
/** A `@multiset_function`: from `multisetFunction(f)`, or the program language's `mf_def`.
 * `f.call(...inputs, { star, ...kwargs })` evaluates it (Python's `f(*inputs, star=star, **kwargs)`); `star` may
 * be left out (then it is inferred), the other options are the function's keyword-only arguments. */
export class MultisetFunction extends Handle {
  call(...rest) {
    const [args, { star, ...kwargs }] = opts(rest);
    // `star` left out (undefined) is inferred; any other value is passed, as Python's `star=...` (even null).
    const items = Object.entries(kwargs);
    if (star !== undefined) items.unshift(["star", star]);
    return call("mf_call_kw", { f: this, args: List.from(args), kwargs: List.from(items) });
  }
  /** The same as `call` (Python's `evaluator.evaluate(...)`). */
  evaluate(...rest) {
    return this.call(...rest);
  }
}

/** `@multiset_function`: a multiset evaluator from a JS function of multisets, as Python's decorator makes one.
 *
 * Calling the result traces `f`: it is called with one parameter expression per input (a `MultisetExpression`,
 * or a `MultisetTupleExpression` for a `MultiDeal` input, unless `star` unpacks the first input into its hands),
 * builds expressions with their methods, and returns an evaluation of them (`a.sub(b).size()`, a JS evaluator's
 * `evaluate(a)`, another multiset function's `call(a)`), an array of evaluations (a joint evaluation, whose
 * outcomes are arrays), or a constant. Evaluations inside `f` only record what to evaluate: the core evaluates
 * the whole function at once. `f` may be traced several times, so it should be a pure function.
 *
 * Python reads the signature from the function; here `params` (default `f.length`) is the number of required
 * positional parameters, `rest` says `f` takes any number more (`(...xs) => ...`), and `kwonly` names its
 * keyword-only parameters, which `f` receives as a trailing options object: `(a, { target } = {}) => ...` with
 * `{ kwonly: ["target"] }` (give it a default, as for other callbacks, or `f.length` counts it). */
export function multisetFunction(f, { params = f.length, rest = false, kwonly = [] } = {}) {
  if (typeof f !== "function") throw new TypeError("icepool: multisetFunction takes a function");
  const args = `{"f":${enc(f)},"params":${Number(params)},"var":${rest ? "true" : "false"},"kwonly":${JSON.stringify(Array.from(kwonly, String))}}`;
  return dec(unwrap(wasm(() => W.multiset_function(args))));
}
class RawResult extends Handle {}

Object.assign(KINDS, {
  die: Die,
  pool: Pool,
  deck: Deck,
  deal: Deal,
  multi_deal: MultiDeal,
  tuple_expr: MultisetTupleExpression,
  expr: Expression,
  again: AgainExpression,
  fn: CoreFunction,
  mfunc: MultisetFunction,
  raw: RawResult,
});

let againHandle;
/** `icepool.Again`, for `new Die([1, 2, Again.add(3)], { againDepth: 2 })`. Made on first use. */
export const Again = Object.create(AgainExpression.prototype, {
  _h: {
    get() {
      againHandle ??= call("Again");
      return againHandle._h;
    },
  },
  // The one shared `Again` stays (a handle made again on use would be a different expression object anyway).
  release: { value() {} },
});
