// harness/catalogue/functions.py and evaluators.py written in JS, so that the fixtures run their `fn` and
// `evaluate` nodes through JS callbacks and JS evaluators: the oracle then checks the callback machinery too.
// Each body is the Python one with Python's operators (`ops.*`, which run in the core), so outcome types behave as
// in Python; fixed-arity lambdas check their argument count as Python does.

import * as ip from "../dist/node.js";

const { ops } = ip;

/** Python's `bool(x)`. */
export function pyTruth(x) {
  if (x === null || x === undefined) return false;
  switch (typeof x) {
    case "boolean":
      return x;
    case "number":
      return x !== 0; // NaN is truthy, as in Python
    case "bigint":
      return x !== 0n;
    case "string":
      return x.length > 0;
  }
  if (Array.isArray(x)) return x.length > 0;
  if (x instanceof ip.Float) return x.value !== 0;
  if (x instanceof ip.Fraction) return x.numerator !== 0n;
  if (x instanceof ip.DieWithTruth) return x.truth();
  return ip.truth(x); // Vector, Symbols, a plain die (raises), ...
}

const eq = (a, b) => pyTruth(ops.eq(a, b));
const lt = (a, b) => pyTruth(ops.lt(a, b));
const gt = (a, b) => pyTruth(ops.gt(a, b));
const le = (a, b) => pyTruth(ops.le(a, b));
const ge = (a, b) => pyTruth(ops.ge(a, b));
/** Builtin `max(a, b)` / `min(a, b)`: the second only if strictly greater / less. */
const max = (a, b) => (gt(b, a) ? b : a);
const min = (a, b) => (lt(b, a) ? b : a);
const sum = (xs) => xs.reduce((acc, x) => ops.add(acc, x), 0);

/** A Python lambda's fixed arity: anything else is a TypeError at the call. */
function arity(args, n) {
  if (args.length !== n) throw new TypeError(`takes ${n} positional arguments but ${args.length} were given`);
}

/** `t[0]`: a tuple's, Vector's or str's first item. */
function first(t) {
  if (t instanceof ip.Vector) {
    // A VectorWithTruthOnly (no items) cannot be indexed.
    if (t.items === null) throw new TypeError("object is not subscriptable");
    t = t.items;
  }
  if (!Array.isArray(t) && typeof t !== "string") throw new TypeError("object is not subscriptable");
  if (t.length === 0) throw new RangeError("IndexError: index out of range");
  return t[0];
}

/** khepri's `read_roll(roll, success_value)` (harness/core/khepri/f2f.py). */
function readRoll(roll, successValue) {
  arity(arguments, 2);
  if (gt(successValue, 20)) {
    roll = ops.add(roll, ops.sub(successValue, 20));
    successValue = 20;
  }
  if (eq(roll, successValue) || gt(roll, 20)) return 20;
  return lt(roll, successValue) ? roll : 0;
}

/** `FUNCTIONS[name](*params)`, except "mapping" (a dict: the core's own catalogue makes it). */
export const FUNCTIONS = {
  add_k: (k) => function (x) { arity(arguments, 1); return ops.add(x, k); },
  mul_k: (k) => function (x) { arity(arguments, 1); return ops.mul(x, k); },
  clamp: (lo, hi) => function (x) { arity(arguments, 1); return min(max(x, lo), hi); },
  read_roll: () => readRoll,
  is_even: () => function (x) { arity(arguments, 1); return ops.eq(ops.mod(x, 2), 0); },
  gt_k: (k) => function (x) { arity(arguments, 1); return ops.gt(x, k); },
  identity: () => function (x) { arity(arguments, 1); return x; },
  swap: () => function (a, b) { arity(arguments, 2); return [b, a]; },
  add2: () => function (a, b) { arity(arguments, 2); return ops.add(a, b); },
  max2: () => function (a, b) { arity(arguments, 2); return max(a, b); },
  mul2: () => function (a, b) { arity(arguments, 2); return ops.mul(a, b); },
  sum3: () => function (a, b, c) { arity(arguments, 3); return ops.add(ops.add(a, b), c); },
  first: () => function (t) { arity(arguments, 1); return first(t); },
  pair_k: (k) => function (x) { arity(arguments, 1); return [x, k]; },
  die_if_k: (k, n) => function (x) { arity(arguments, 1); return eq(x, k) ? ip.d(n) : x; },
  plus_d: (n) => function (x) { arity(arguments, 1); return ops.add(x, ip.d(n)); },
  reroll_below: (k) => function (x) { arity(arguments, 1); return lt(x, k) ? ip.Reroll : x; },
  reroll_in_tuple: (k) => function (x) { arity(arguments, 1); return eq(x, k) ? [x, ip.Reroll] : [x, x]; },
  tuple_d: (n) => function (x) { arity(arguments, 1); return [x, ip.d(n)]; },
  // `lambda *args: v`: no keyword arguments (a trailing options object).
  const: (v) => (...args) => {
    if (args.some((a) => a !== null && typeof a === "object" && a.constructor === Object)) {
      throw new TypeError("<lambda>() got an unexpected keyword argument");
    }
    return v;
  },
  restart_below: (k) => function (x) { arity(arguments, 1); return lt(x, k) ? ip.Restart : x; },
  break_k: (k, v) => function (x) { arity(arguments, 1); return eq(x, k) ? new ip.Break(v) : new ip.Break(); },
  break_eq: () => function (a, b) { arity(arguments, 2); return eq(a, b) ? new ip.Break() : ops.add(a, b); },
  count_diff: () => (outcome, ...counts) => {
    if (counts.length === 0) throw new RangeError("IndexError: tuple index out of range");
    return ops.sub(counts[0], sum(counts.slice(1)));
  },
  count_if_gt_k: (k) => (outcome, ...counts) => (gt(outcome, k) ? sum(counts) : 0),
  // `lambda x, *, k: x + k`: k arrives in the trailing options object (length 1, as Python counts it).
  add_kw: () => function (x, kw = undefined) {
    if (arguments.length !== 2 || !("k" in kw) || Object.keys(kw).length !== 1) {
      throw new TypeError("add_kw() takes 1 positional argument and the keyword-only argument k");
    }
    return ops.add(x, kw.k);
  },
  cap_sum: (cap) => function (total, roll) { arity(arguments, 2); return min(ops.add(total, roll), cap); },
  break_at: (k, v) => function (total, roll) {
    arity(arguments, 2);
    return ge(total, k) ? new ip.Break(v) : ops.add(total, roll);
  },
  restart_on: (k, cap) => function (total, roll) {
    arity(arguments, 2);
    return eq(roll, k) ? ip.Restart : min(ops.add(total, roll), cap);
  },
  walk: (lo, hi) => function (x, step) {
    arity(arguments, 2);
    return le(x, lo) || ge(x, hi) ? x : ops.add(x, step);
  },
  walk_die: (lo, hi) => function (x) {
    arity(arguments, 1);
    return le(x, lo) || ge(x, hi) ? x : ops.add(x, new ip.Die([-1, 1]));
  },
  collatz: () => function (x) {
    arity(arguments, 1);
    if (le(x, 1)) return 1;
    return eq(ops.mod(x, 2), 0) ? ops.floordiv(x, 2) : ops.add(ops.mul(3, x), 1);
  },
  pair_walk: (k) => function (a, b, roll) {
    arity(arguments, 3);
    return ge(ops.add(a, b), k) ? [a, b] : [b, ops.add(a, roll)];
  },
  // map_to_pool's: what `Pool(...)` takes (a Python list is a `List`), or an expression.
  n_of: (n) => function (x) { arity(arguments, 1); return listTimes([x], n); },
  pool_d: (n) => function (x) { arity(arguments, 1); return ip.d(n).pool(x); },
  pair_list: () => function (a, b) { arity(arguments, 2); return ip.List.of(a, b); },
  reroll_or_pair: (k) => function (x) { arity(arguments, 1); return eq(x, k) ? ip.Reroll : ip.List.of(x, k); },
};

/** harness/catalogue/rvs.py: random variables duck-typed like `scipy.stats` ones (`cdf` takes a sequence and
 * returns a list; a continuous one has a `pdf`), with Python's operators. */
const F0 = new ip.Float(0);
const F1 = new ip.Float(1);
export const RVS = {
  /** The ints lo..hi-1 (hi excluded), like `scipy.stats.randint(lo, hi)`. */
  discrete_uniform: (lo, hi) => ({
    cdf: (xs) =>
      Array.from(xs, (x) => min(max(ops.truediv(ops.add(ops.sub(ops.floor(x), lo), 1), ops.sub(hi, lo)), F0), F1)),
  }),
  /** Uniform on [lo, hi]. */
  continuous_uniform: (lo, hi) => ({
    pdf: (x) => (le(lo, x) && le(x, hi) ? ops.truediv(1, ops.sub(hi, lo)) : F0),
    cdf: (xs) => Array.from(xs, (x) => min(max(ops.truediv(ops.sub(x, lo), ops.sub(hi, lo)), F0), F1)),
  }),
};

/** Python's `list * n`: none for `n <= 0`, TypeError for a non-int `n` (a bool is an int). */
function listTimes(items, n) {
  if (typeof n === "boolean") n = Number(n);
  if (typeof n === "bigint") n = Number(n);
  if (typeof n !== "number" || !Number.isInteger(n)) {
    throw new TypeError("can't multiply sequence by non-int");
  }
  const out = ip.List.of();
  for (let i = 0; i < n; i++) out.push(...items);
  return out;
}

/** `MultisetEvaluator.consecutive`: every int from the first outcome to the last. */
function consecutive(outcomes) {
  if (outcomes.length === 0) return [];
  const int = (x) => {
    if (typeof x === "boolean") return BigInt(x);
    if ((typeof x === "number" && Number.isInteger(x)) || typeof x === "bigint") return BigInt(x);
    throw new TypeError("consecutive cannot be used with outcomes of type other than 'int'.");
  };
  outcomes.forEach(int);
  const out = [];
  for (let i = int(outcomes[0]); i <= int(outcomes[outcomes.length - 1]); i++) out.push(i);
  return out;
}

/** `int` iteration (`for _ in x`, `zip`): a tuple's items; an int or None is not iterable. */
function items(x) {
  if (!Array.isArray(x)) throw new TypeError("object is not iterable");
  return x;
}

/** `EVALUATORS[name](*params)` as JS evaluators. */
export const EVALUATORS = {
  count_eq: (k) => ip.evaluator({
    initialState: () => 0,
    nextState(state, order, outcome, ...counts) {
      arity(counts, 1);
      return eq(outcome, k) ? ops.add(state, counts[0]) : state;
    },
  }),
  reroll_at_least: (k) => ip.evaluator({
    nextState(state, order, outcome, ...counts) {
      arity(counts, 1);
      if (ge(counts[0], k)) return ip.Reroll;
      return ops.add(pyTruth(state) ? state : 0, counts[0]);
    },
  }),
  final_die: (n) => ip.evaluator({
    initialState: () => 0,
    nextState(state, order, outcome, ...counts) {
      arity(counts, 1);
      return ops.add(state, counts[0]);
    },
    finalOutcome(finalState) {
      if (eq(finalState, 0)) return ip.Reroll;
      if (lt(finalState, 0)) return null;
      if (le(finalState, n)) return ip.d(finalState);
      return finalState;
    },
  }),
  seen: (req) => ip.evaluator({
    initialState(order) {
      if (eq(req, 2) || (eq(req, 1) && order < 0)) throw new ip.UnsupportedOrder();
      return [];
    },
    nextState(state, order, outcome, ...counts) {
      if (eq(req, -1) && order > 0) throw new ip.UnsupportedOrder();
      return counts.some((c) => gt(c, 0)) ? ops.add(state, [outcome]) : state;
    },
  }),
  count_diff: () => ip.evaluator({
    initialState: () => 0,
    nextState(state, order, outcome, ...counts) {
      if (counts.length === 0) throw new RangeError("IndexError: tuple index out of range");
      return ops.add(state, ops.mul(outcome, ops.sub(counts[0], sum(counts.slice(1)))));
    },
    finalOutcome: (finalState, order, outcomes, ...sizes) => [finalState, ...sizes],
  }),
  zero_runs: () => ip.evaluator({
    extraOutcomes: consecutive,
    initialState: () => 0,
    nextState(state, order, outcome, ...counts) {
      arity(counts, 1);
      return ops.add(state, eq(counts[0], 0) ? 1 : 0);
    },
  }),
  // `initial_state(..., *sizes, start=0, scale=1)` / `final_outcome(..., *sizes, start=0, scale=1)`.
  scaled_size: () => {
    const kwargs = (rest) => {
      const last = rest[rest.length - 1];
      const kw = last !== null && typeof last === "object" && !Array.isArray(last) && last.constructor === Object ? last : {};
      for (const k of Object.keys(kw)) {
        if (k !== "start" && k !== "scale") throw new TypeError(`unexpected keyword argument '${k}'`);
      }
      return { start: 0, scale: 1, ...kw };
    };
    return ip.evaluator({
      initialState: (order, outcomes, ...rest) => kwargs(rest).start,
      nextState: (state, order, outcome, ...counts) => ops.add(state, sum(counts)),
      finalOutcome: (finalState, order, outcomes, ...rest) => ops.mul(finalState, kwargs(rest).scale),
    });
  },
  hand_sums: () => ip.evaluator({
    initialState(order, outcomes, ...sizes) {
      if (sizes.length === 0) throw new RangeError("IndexError: tuple index out of range");
      return items(sizes[0]).map(() => 0);
    },
    nextState(state, order, outcome, ...counts) {
      arity(counts, 1);
      const cs = items(counts[0]);
      const st = items(state);
      return st.slice(0, Math.min(st.length, cs.length)).map((x, i) => ops.add(x, ops.mul(outcome, cs[i])));
    },
  }),
  record: () => ip.evaluator({
    nextState: (state, order, outcome, ...counts) => ops.add(pyTruth(state) ? state : [], [[outcome, ...counts]]),
    finalOutcome: (finalState, order, outcomes, ...sizes) => [finalState, sizes],
  }),
};
