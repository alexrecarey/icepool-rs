// `icepool.Again` (population/again.py): an expression object standing for "roll again", used as an outcome of
// `new Die(..., { againDepth })`. INTERIM (task C6): held through the bridge (rpc.ts).

import type { DieWithTruth, Operand } from "./die.js";
import { HANDLE, Handle, call, registerKind } from "./rpc.js";
import type { Int } from "./values.js";

type BinaryName =
  | "add" | "sub" | "mul" | "truediv" | "floordiv" | "mod" | "pow" | "lshift" | "rshift" | "and" | "or" | "xor"
  | "lt" | "le" | "gt" | "ge";
const BINARY: BinaryName[] = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor", "lt", "le", "gt", "ge"];
const UNARY = ["neg", "pos", "abs", "invert"] as const;

export interface AgainExpression {
  add(other: Operand): AgainExpression;
  sub(other: Operand): AgainExpression;
  mul(other: Operand): AgainExpression;
  truediv(other: Operand): AgainExpression;
  floordiv(other: Operand): AgainExpression;
  mod(other: Operand): AgainExpression;
  pow(other: Operand): AgainExpression;
  lshift(other: Operand): AgainExpression;
  rshift(other: Operand): AgainExpression;
  and(other: Operand): AgainExpression;
  or(other: Operand): AgainExpression;
  xor(other: Operand): AgainExpression;
  radd(other: Operand): AgainExpression;
  rsub(other: Operand): AgainExpression;
  rmul(other: Operand): AgainExpression;
  rtruediv(other: Operand): AgainExpression;
  rfloordiv(other: Operand): AgainExpression;
  rmod(other: Operand): AgainExpression;
  rpow(other: Operand): AgainExpression;
  rlshift(other: Operand): AgainExpression;
  rrshift(other: Operand): AgainExpression;
  rand(other: Operand): AgainExpression;
  ror(other: Operand): AgainExpression;
  rxor(other: Operand): AgainExpression;
  lt(other: Operand): AgainExpression;
  le(other: Operand): AgainExpression;
  gt(other: Operand): AgainExpression;
  ge(other: Operand): AgainExpression;
  neg(): AgainExpression;
  pos(): AgainExpression;
  abs(): AgainExpression;
  invert(): AgainExpression;
}

/** An `AgainExpression`: `Again`, or operators applied to it. */
export class AgainExpression extends Handle {
  /** `==`: an expression, with a truth value (`truth(expr)`) as Python's. */
  eq(other: Operand): AgainExpression | DieWithTruth {
    return call("eq", { a: this, b: other }) as AgainExpression;
  }
  ne(other: Operand): AgainExpression | DieWithTruth {
    return call("ne", { a: this, b: other }) as AgainExpression;
  }
  /** `truediv`. */
  div(other: Operand): AgainExpression {
    return this.truediv(other);
  }
  /** `n @ expr` / `expr @ n`. */
  matmul(other: Int | Operand): AgainExpression {
    return call("matmul", { n: this, die: other }) as AgainExpression;
  }
  rmatmul(n: Int | Operand): AgainExpression {
    return call("matmul", { n, die: this }) as AgainExpression;
  }
}

{
  const proto = AgainExpression.prototype as unknown as Record<string, unknown>;
  for (const op of BINARY) {
    proto[op] = function (this: AgainExpression, other: unknown) {
      return call(op, { a: this, b: other });
    };
  }
  // The reflected arithmetic operators: `3 - Again` is `Again.rsub(3)`.
  for (const op of BINARY.slice(0, 12)) {
    proto[`r${op}`] = function (this: AgainExpression, other: unknown) {
      return call(op, { a: other, b: this });
    };
  }
  for (const op of UNARY) {
    proto[op] = function (this: AgainExpression) {
      return call(op, { a: this });
    };
  }
}

registerKind("again", (h) => new AgainExpression(HANDLE, h));

/** The one shared `Again`, made on first use; `free()` leaves it. */
class SharedAgain extends AgainExpression {
  protected override get h(): number {
    if (this.handle < 0) this.adopt(call("Again"), AgainExpression);
    return this.handle;
  }
  override free(): void {}
}

/** `icepool.Again`, for `new Die([1, 2, Again.add(3)], { againDepth: 2 })`. */
export const Again: AgainExpression = new SharedAgain(HANDLE, -1);
