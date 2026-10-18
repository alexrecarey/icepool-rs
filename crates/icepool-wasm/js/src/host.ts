// What the typed exports call back for (src/api/host.rs): JS's value classes, made and taken apart for the core.

import { makeError } from "./errors.js";
import { Complex, Float, Fraction, List, Opaque, StateKey, Symbols, Vector, type Int, type Outcome } from "./values.js";
import { HOST } from "./wasm.js";

Object.assign(HOST, {
  /** A JS object as `[tag, ...parts]` (src/api/host.rs `describe`). */
  describe(x: object): unknown[] {
    if (Array.isArray(x)) return [x instanceof List ? "list" : "array"];
    if (x instanceof Float) return ["float", x.value];
    if (x instanceof Fraction) return ["fraction", x.numerator, x.denominator];
    if (x instanceof Complex) return ["complex", x.re, x.im];
    if (x instanceof Vector) return ["vector", x.items, x.hasTruth, x.truthValue];
    if (x instanceof Symbols) return ["symbols", x.items, x.str];
    if (x instanceof StateKey) return ["state", x.key];
    return ["other", x.constructor?.name ?? "object"];
  },
  makeFloat: (x: number) => new Float(x),
  makeFraction: (n: bigint, d: bigint) => new Fraction(n, d),
  makeComplex: (re: number, im: number) => new Complex(re, im),
  makeVector: (items: Outcome[] | null, hasTruth: boolean, truth: Outcome) => Vector.fromCore(items, hasTruth, truth),
  makeSymbols: (items: [string, Int][], str: string | undefined) => Symbols.fromCore(items, str ?? null),
  makeOpaque: (repr: string) => new Opaque(repr),
  /** A JS evaluator's state that was a plain object, from its canonical JSON. */
  makeState: (key: string): unknown => JSON.parse(key),
  makeError,
});
