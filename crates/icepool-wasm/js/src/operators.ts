// Python's operators on any operands (`a + b` is `binary("add", a, b)`): the dispatch Python does between dice,
// Again expressions and plain values. A die on either side is a die operation (die.ts); two plain values are the
// core's `Value` operators; what remains (Again expressions, Python's odd corners) goes through the bridge.

import { Die } from "./die.js";
import { call } from "./rpc.js";
import { Symbols, isPlainValue, type Int } from "./values.js";
import { raw, wasm } from "./wasm.js";

export type BinaryName =
  | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
  | "lt" | "le" | "gt" | "ge" | "eq" | "ne";
export type UnaryName = "neg" | "pos" | "abs" | "invert" | "trunc" | "floor" | "ceil";

/** The core's operator for each name. */
const BINARY = {
  add: "Add", sub: "Sub", mul: "Mul", truediv: "TrueDiv", floordiv: "FloorDiv", mod: "Mod", pow: "Pow",
  lshift: "LShift", rshift: "RShift", and: "And", or: "Or", xor: "Xor", lt: "Lt", le: "Le", gt: "Gt", ge: "Ge",
  eq: "Eq", ne: "Ne",
} as const;
const UNARY = { neg: "Neg", pos: "Pos", invert: "Invert", abs: "Abs", trunc: "Trunc", floor: "Floor", ceil: "Ceil" } as const;

/** With a die only on the right, the left operand's own operator does not know dice, so Python calls the die's
 * reflected one: comparisons swap sides (`3 < die` is `die > 3`), `==` / `!=` too, the arithmetic operators keep
 * the order (`3 - die` is `die.__rsub__(3)`). */
const REFLECTED = { lt: "gt", le: "ge", gt: "lt", ge: "le", eq: "eq", ne: "ne" } as const;

/** `a <op> b` with Python's semantics. */
export function binary(op: BinaryName, a: unknown, b: unknown): unknown {
  if (a instanceof Die) return a[op](b as never);
  if (b instanceof Die) {
    // Except that `str % die` is printf-style formatting, and `Symbols // die` counts the die as a mapping: the
    // left operand's own operator does know those. The bridge has them.
    const leftKnows = (op === "mod" && typeof a === "string") || (op === "floordiv" && a instanceof Symbols);
    if (!leftKnows) {
      if (op in REFLECTED) return b[REFLECTED[op as keyof typeof REFLECTED]](a as never);
      const reflected = `r${op}` as `r${Exclude<BinaryName, keyof typeof REFLECTED>}`;
      return b[reflected](a as never);
    }
  }
  if (isPlainValue(a) && isPlainValue(b)) {
    const code = raw().BinaryOp[BINARY[op]];
    return wasm((w) => w.valueBinary(code, a, b));
  }
  return call(op, { a, b });
}

/** `<op> a` with Python's semantics. */
export function unary(op: UnaryName, a: unknown): unknown {
  if (a instanceof Die) return a[op]();
  if (isPlainValue(a)) {
    const code = raw().UnaryOp[UNARY[op]];
    return wasm((w) => w.valueUnary(code, a));
  }
  return call(op, { a });
}

/** `round(a, ndigits)` with Python's semantics. */
export function round(a: unknown, ndigits: Int | null = null): unknown {
  if (a instanceof Die) return a.round(ndigits);
  if (isPlainValue(a) && isPlainValue(ndigits)) return wasm((w) => w.valueRound(a, ndigits));
  return call("round", { a, ndigits });
}
