// Python's operators on any operands (`a + b` is `binary("add", a, b)`): the dispatch Python does between dice,
// Again expressions and plain values. A die on either side is a die operation (die.ts), an Again expression on
// either side a bigger expression (again.ts); two plain values are the core's `Value` operators; `Reroll` is an
// enum member, which only has `==`; pools, multiset expressions and decks have their own operators (multiset.ts,
// deck.ts), which Python calls before the other operand's reflected ones.

import { AgainExpression } from "./again.js";
import { Die } from "./die.js";
import { IcepoolError } from "./errors.js";
import { MultisetExpression, isDeck, isMultisetExpression, sameExpression, type TupleExpressionLike } from "./multiset.js";
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

/** Python's `isinstance(x, int)` (a bool too). */
const isInt = (x: unknown): boolean => typeof x === "bigint" || typeof x === "boolean" || Number.isInteger(x);

/** The reflected method of `op` on a die or an Again expression (`rsub` for `sub`, `gt` for `lt`). */
function reflected(op: BinaryName): string {
  return op in REFLECTED ? REFLECTED[op as keyof typeof REFLECTED] : `r${op}`;
}

/** The methods for Python's `a.__op__(b)` of a multiset expression (`__le__` is `issubset`, `__ge__`
 * `issuperset`). */
const EXPRESSION_OPERATORS: Partial<Record<BinaryName, string>> = {
  add: "add", sub: "sub", and: "and", or: "or", xor: "xor", mul: "mul", floordiv: "floordiv", mod: "mod",
  lt: "lt", le: "issubset", gt: "gt", ge: "issuperset", eq: "eq", ne: "ne",
};
/** The methods for Python's `b.__rop__(a)` of a multiset expression: comparisons swap sides, `__rmul__` is
 * `__mul__` (counts commute), there is no `__rmod__`. */
const EXPRESSION_REFLECTED: Partial<Record<BinaryName, string>> = {
  add: "radd", sub: "rsub", and: "rand", or: "ror", xor: "rxor", mul: "mul", floordiv: "rfloordiv",
  lt: "gt", le: "issuperset", gt: "lt", ge: "issubset", eq: "eq", ne: "ne",
};
/** A deck's operators (population/deck.py): no comparisons but `==`, no `__rxor__`, `__rfloordiv__`, `__rmod__`. */
const DECK_OPERATORS: Partial<Record<BinaryName, string>> = {
  add: "add", sub: "sub", and: "and", or: "or", xor: "xor", mul: "mul", floordiv: "floordiv", mod: "mod", eq: "eq", ne: "ne",
};
const DECK_REFLECTED: Partial<Record<BinaryName, string>> = { add: "radd", sub: "rsub", and: "rand", or: "ror", mul: "rmul", eq: "eq", ne: "ne" };

/** The method of `x` for `x.__op__(y)` (`reflected`: `x.__rop__(y)`), if `x` is a multiset expression or a deck
 * that has one. A multiset tuple expression has no operators. */
function ownOperator(x: unknown, op: BinaryName, reflected: boolean): ((y: unknown) => unknown) | undefined {
  const table = x instanceof MultisetExpression
    ? (reflected ? EXPRESSION_REFLECTED : EXPRESSION_OPERATORS)
    : isDeck(x) ? (reflected ? DECK_REFLECTED : DECK_OPERATORS) : {};
  const name = table[op];
  return name === undefined ? undefined : (y) => (x as unknown as Record<string, (y: unknown) => unknown>)[name](y);
}

/** Whether `a.__op__` answers before even an Again expression's reflected operator, never returning
 * `NotImplemented`: a deck's `+ - & | ^` count the other operand (`Counter(other)`), a multiset expression's `//` is
 * `count_subset` for anything but an int, and a deck's or multiset tuple expression's `==` is
 * `MaybeHashKeyed.__eq__`, a plain bool. */
function answersFirst(op: BinaryName, a: unknown): boolean {
  if (op === "eq" || op === "ne") return isDeck(a) || (isMultisetExpression(a) && !(a instanceof MultisetExpression));
  if (isDeck(a)) return op === "add" || op === "sub" || op === "and" || op === "or" || op === "xor";
  return op === "floordiv" && a instanceof MultisetExpression;
}

/** `a <op> b` with Python's semantics. */
export function binary(op: BinaryName, a: unknown, b: unknown): unknown {
  if (a instanceof Die || a instanceof AgainExpression) return (a[op] as (x: unknown) => unknown)(b);
  if (answersFirst(op, a)) {
    // A multiset tuple expression's `equals`: the same `hash_key` (only another expression's can match).
    if (!isDeck(a) && !(a instanceof MultisetExpression)) {
      return (isMultisetExpression(b) && sameExpression(a as TupleExpressionLike, b as TupleExpressionLike)) !== (op === "ne");
    }
    return (ownOperator(a, op, false) as (y: unknown) => unknown)(b);
  }
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
  // A multiset expression's or deck's own operator, which does Python's whole dispatch (it calls the other
  // operand's reflected one where Python's returns NotImplemented). A deck's `* // %` take an int only: a die on
  // the right answers them.
  const own = ownOperator(a, op, false);
  if (own !== undefined && !(isDeck(a) && b instanceof Die)) return own(b);
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
  // On the right, a multiset expression's or deck's reflected operator: an expression's `__rmul__` only takes an
  // int, as its `__mul__` returns NotImplemented for anything else.
  const reflectedOwn = ownOperator(b, op, true);
  if (reflectedOwn !== undefined && !(op === "mul" && b instanceof MultisetExpression && !isInt(a))) return reflectedOwn(a);
  if (isMultisetExpression(a) || isMultisetExpression(b) || isDeck(a) || isDeck(b)) {
    // `str % x` is printf-style formatting, and these are mappings to it (they have `__getitem__`): without
    // conversion specifiers the result is the string itself.
    if (op === "mod" && typeof a === "string") {
      if (a.includes("%")) throw new IcepoolError("Unported", "NotImplementedError", "str % formatting with conversion specifiers");
      return a;
    }
    // `object.__eq__`: identity.
    if (op === "eq" || op === "ne") return (a === b) !== (op === "ne");
    throw unsupported(op, a, b);
  }
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
  // `MultisetExpression.__neg__` and `__pos__`; it has no other unary operator, a deck none.
  if (a instanceof MultisetExpression && (op === "neg" || op === "pos")) return a[op]();
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
  // No outcome type has `@`, nor does a pool, an expression or a deck.
  throw typeError(`unsupported operand type(s) for @: '${pyTypeName(n)}' and '${pyTypeName(die)}'`);
}

/** `round(a, ndigits)` with Python's semantics. */
export function round(a: unknown, ndigits: Int | null = null): unknown {
  if (a instanceof Die) return a.round(ndigits);
  if (a instanceof AgainExpression) throw new IcepoolError("WrongType", "TypeError", "type AgainExpression doesn't define __round__ method");
  if (isPlainValue(a) && isPlainValue(ndigits)) return wasm((w) => w.valueRound(a, ndigits));
  if (!isPlainValue(a)) throw typeError(`type ${pyTypeName(a)} doesn't define __round__ method`);
  throw typeError(`'${pyTypeName(ndigits)}' object cannot be interpreted as an integer`);
}
