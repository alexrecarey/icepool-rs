// Python's operators on any operands (`a + b` is `binary("add", a, b)`): the dispatch Python does between dice,
// Again expressions and plain values. A die on either side is a die operation (die.ts), an Again expression on
// either side a bigger expression (again.ts); two plain values are the core's `Value` operators; `Reroll` is an
// enum member, which only has `==`. Pools, decks and multiset expressions without a die still go through the
// bridge (task C6l).

import { AgainExpression } from "./again.js";
import { Die } from "./die.js";
import { IcepoolError } from "./errors.js";
import { Handle, call } from "./rpc.js";
import { Reroll, Restart, Symbols, isPlainValue, pyTypeName, type Int } from "./values.js";
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

/** With a die (or an Again expression) only on the right, the left operand's own operator does not know it, so
 * Python calls the right operand's reflected one: comparisons swap sides (`3 < die` is `die > 3`), `==` / `!=` too,
 * the arithmetic operators keep the order (`3 - die` is `die.__rsub__(3)`). */
const REFLECTED = { lt: "gt", le: "ge", gt: "lt", ge: "le", eq: "eq", ne: "ne" } as const;

/** Python's spelling of each operator, for its `TypeError`s. */
const SYMBOL: Record<BinaryName, string> = {
  add: "+", sub: "-", mul: "*", truediv: "/", floordiv: "//", mod: "%", pow: "** or pow()", lshift: "<<", rshift: ">>",
  and: "&", or: "|", xor: "^", lt: "<", le: "<=", gt: ">", ge: ">=", eq: "==", ne: "!=",
};

const typeError = (message: string) => new IcepoolError("WrongType", "TypeError", message);

/** Python's `TypeError` for operands that do not support `op`. */
function unsupported(op: BinaryName, a: unknown, b: unknown): IcepoolError {
  const types = `'${pyTypeName(a)}' and '${pyTypeName(b)}'`;
  if (op === "lt" || op === "le" || op === "gt" || op === "ge") return typeError(`'${SYMBOL[op]}' not supported between instances of ${types}`);
  return typeError(`unsupported operand type(s) for ${SYMBOL[op]}: ${types}`);
}

const isRerollType = (x: unknown) => x === Reroll || x === Restart;

/** The reflected method of `op` on a die or an Again expression (`rsub` for `sub`, `gt` for `lt`). */
function reflected(op: BinaryName): string {
  return op in REFLECTED ? REFLECTED[op as keyof typeof REFLECTED] : `r${op}`;
}

/** `a <op> b` with Python's semantics. */
export function binary(op: BinaryName, a: unknown, b: unknown): unknown {
  if (a instanceof Die || a instanceof AgainExpression) return (a[op] as (x: unknown) => unknown)(b);
  if (b instanceof AgainExpression) {
    // Except that `str % x` is printf-style formatting, which runs before the expression's `__rmod__`. The
    // expression is neither a tuple nor a mapping, so it is the one argument: without a conversion specifier it is
    // left over. (With one, `%s` would format its repr, which holds an address.)
    if (op === "mod" && typeof a === "string") {
      if (a.includes("%")) throw new IcepoolError("Unported", "NotImplementedError", "str % Again with conversion specifiers");
      throw new IcepoolError("WrongType", "TypeError", "not all arguments converted during string formatting");
    }
    return (b as unknown as Record<string, (x: unknown) => unknown>)[reflected(op)](a);
  }
  if (b instanceof Die) {
    // Except that `str % die` is printf-style formatting, and `Symbols // die` counts the die as a mapping: the
    // left operand's own operator does know those.
    if (op === "mod" && typeof a === "string") {
      // A mapping argument is never "not all converted", so without conversion specifiers the result is the
      // string itself.
      if (a.includes("%")) throw new IcepoolError("Unported", "NotImplementedError", "str % formatting with conversion specifiers");
      return a;
    }
    // `count_subset` of the die's quantities.
    if (op === "floordiv" && a instanceof Symbols) return wasm((w) => w.symbolsMethod(a, "count_subset_mapping", b.items()));
    return (b as unknown as Record<string, (x: unknown) => unknown>)[reflected(op)](a);
  }
  if (isPlainValue(a) && isPlainValue(b)) {
    const code = raw().BinaryOp[BINARY[op]];
    return wasm((w) => w.valueBinary(code, a, b));
  }
  if (a instanceof Handle || b instanceof Handle) return call(op, { a, b });
  // `Reroll` is equal only to itself, and has no other operator.
  if ((isRerollType(a) || isPlainValue(a)) && (isRerollType(b) || isPlainValue(b))) {
    if (op === "eq" || op === "ne") return (a === b) !== (op === "ne");
    throw unsupported(op, a, b);
  }
  throw new IcepoolError("Unported", "NotImplementedError", "operators on lists, tuples of dice, functions");
}

/** `<op> a` with Python's semantics. */
export function unary(op: UnaryName, a: unknown): unknown {
  if (a instanceof Die) return a[op]();
  if (a instanceof AgainExpression) {
    // `AgainExpression` has `-`, `+`, `~` and `abs`, but no `__trunc__`, `__floor__`, `__ceil__`.
    if (op === "trunc" || op === "floor" || op === "ceil") {
      throw new IcepoolError("WrongType", "TypeError", `type AgainExpression doesn't define __${op}__ method`);
    }
    return a[op]();
  }
  if (isPlainValue(a)) {
    const code = raw().UnaryOp[UNARY[op]];
    return wasm((w) => w.valueUnary(code, a));
  }
  if (a instanceof Handle) return call(op, { a });
  const type = pyTypeName(a);
  if (op === "trunc" || op === "floor" || op === "ceil") throw typeError(`type ${type} doesn't define __${op}__ method`);
  if (op === "abs") throw typeError(`bad operand type for abs(): '${type}'`);
  throw typeError(`bad operand type for unary ${{ neg: "-", pos: "+", invert: "~" }[op]}: '${type}'`);
}

/** `n @ die` with Python's semantics: a die on either side sums rolls; an Again expression on either side (dice
 * return `NotImplemented` for one) makes a bigger expression. */
export function matmul(n: unknown, die: unknown): unknown {
  if (n instanceof AgainExpression) return n.matmul(die as Int);
  if (die instanceof AgainExpression) return die.rmatmul(n as Int);
  if (die instanceof Die) return die.rmatmul(n as Int | Die);
  if (n instanceof Die) return n.matmul(die as Int);
  if (n instanceof Handle || die instanceof Handle) return call("matmul", { n, die });
  // No outcome type has `@`.
  throw typeError(`unsupported operand type(s) for @: '${pyTypeName(n)}' and '${pyTypeName(die)}'`);
}

/** `round(a, ndigits)` with Python's semantics. */
export function round(a: unknown, ndigits: Int | null = null): unknown {
  if (a instanceof Die) return a.round(ndigits);
  if (a instanceof AgainExpression) throw new IcepoolError("WrongType", "TypeError", "type AgainExpression doesn't define __round__ method");
  if (isPlainValue(a) && isPlainValue(ndigits)) return wasm((w) => w.valueRound(a, ndigits));
  if (a instanceof Handle || (ndigits as unknown) instanceof Handle) return call("round", { a, ndigits });
  if (!isPlainValue(a)) throw typeError(`type ${pyTypeName(a)} doesn't define __round__ method`);
  throw typeError(`'${pyTypeName(ndigits)}' object cannot be interpreted as an integer`);
}
