// Runs fixture programs (fixtures/**/*.jsonl: programs with the oracle's results) through the JS API.
//
// A program is evaluated the way harness/core/grammar_api.py `evaluate` does, but every node becomes a JS call:
// the idiomatic method where the node has one (`add` with a die on the left is `a.add(b)`, `reroll` is
// `die.reroll(which, { star, depth })`, ...), else the generic `call(op, args)`. Every intermediate value is a JS
// value (dice are JS objects, tuples arrays, ints numbers or BigInts...), so each node's arguments cross the JS API
// boundary both ways. Catalogue functions and evaluators are the JS ones of catalogue.mjs (JS callbacks), except
// the "mapping" dict. The result is encoded back into the wire format by JS (dice through `items()`) and compared
// exactly with the oracle's, as harness/core/compare.py does.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as ip from "../node.js";
import { EVALUATORS, FUNCTIONS } from "./catalogue.mjs";

/** Each node's argument names (harness/grammar/, by `run`'s signature). Regenerate when nodes are added:
 *  uv run python -c "import inspect, json; from harness.core.grammar_api import NODES, load_grammar; load_grammar();
 *  print(json.dumps({op: sorted(inspect.signature(n.run).parameters) for op, n in sorted(NODES.items())}))" */
const NODE_ARGS = JSON.parse(fs.readFileSync(new URL("./node-args.json", import.meta.url), "utf8"));

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");

/** JSON.parse keeping integers exact (BigInt), as Python's json does. */
export function parseExact(text) {
  return JSON.parse(text, (key, value, context) => {
    if (typeof value !== "number") return value;
    if (!context || context.source === undefined) throw new Error("this JS engine lacks JSON.parse source text access");
    return /^-?\d+$/.test(context.source) ? BigInt(context.source) : value;
  });
}

/** JSON text with BigInts as plain digits and object keys sorted (compare.py's `canon`). */
export function canon(x) {
  if (typeof x === "bigint") return x.toString();
  if (Array.isArray(x)) return `[${x.map(canon).join(",")}]`;
  if (x !== null && typeof x === "object") {
    return `{${Object.keys(x).sort().map((k) => `${JSON.stringify(k)}:${canon(x[k])}`).join(",")}}`;
  }
  return JSON.stringify(x);
}

/** The program's JSON text again (for the lazy arguments). */
function text(x) {
  if (typeof x === "bigint") return x.toString();
  if (Array.isArray(x)) return `[${x.map(text).join(",")}]`;
  if (x !== null && typeof x === "object") {
    return `{${Object.entries(x).map(([k, v]) => `${JSON.stringify(k)}:${text(v)}`).join(",")}}`;
  }
  return JSON.stringify(x);
}

// --- Evaluation ---------------------------------------------------------------------------------------------------

const LAZY = new Set(["khepri", "mf_def"]);

/** Which ops went through an idiomatic method / the generic call (for the summary). */
export const usage = { idiomatic: new Map(), generic: new Map() };
const count = (m, op) => m.set(op, (m.get(op) ?? 0) + 1);

export function evaluate(x) {
  if (Array.isArray(x)) return ip.List.from(x.map(evaluate));
  if (x !== null && typeof x === "object" && "op" in x) {
    const op = x.op;
    const args = {};
    // Arguments in the order the Rust runner evaluates them (serde_json's sorted keys).
    for (const k of Object.keys(x).filter((k) => k !== "op").sort()) {
      if (LAZY.has(op)) args[k] = new ip.RawJSON(text(x[k]));
      else if (op === "fn" && k === "name") args[k] = x[k]; // lazy: the catalogue name as a str
      else args[k] = evaluate(x[k]);
    }
    // A method only for well-formed calls: the node's exact argument names (else Python's TypeError via call()).
    const wellFormed = canon(Object.keys(args).sort()) === canon(NODE_ARGS[op] ?? null);
    const idiomatic = !LAZY.has(op) && wellFormed && IDIOMATIC[op]?.(args);
    if (idiomatic !== undefined && idiomatic !== false) {
      count(usage.idiomatic, op);
      return idiomatic.value;
    }
    count(usage.generic, op);
    return ip.call(op, args);
  }
  // A typed literal, read by the core's own decoder, then held as a JS value.
  return ip.fromWire(text(x));
}

// --- The idiomatic routes: op -> args -> { value } (or undefined when the arguments do not fit the method) --------

const isDie = (x) => x instanceof ip.Die;
const isOperand = (x) => x instanceof ip.Die || x instanceof ip.AgainExpression;
const isKeep = (x) => x instanceof ip.Pool || x instanceof ip.Deal;
const isExpr = (x) => x instanceof ip.MultisetExpression;
const isList = (x) => x instanceof ip.List;
const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
const ok = (value) => ({ value });

const BINARY = ["add", "sub", "mul", "truediv", "floordiv", "mod", "pow", "lshift", "rshift", "and", "or", "xor"];
const COMPARE = ["lt", "le", "gt", "ge", "eq", "ne"];
const UNARY = ["neg", "pos", "abs", "invert", "trunc", "floor", "ceil"];

const IDIOMATIC = {
  ...Object.fromEntries(
    [...BINARY, ...COMPARE].map((op) => [op, ({ a, b }) => ok(isOperand(a) ? a[op](b) : ip.ops[op](a, b))]),
  ),
  ...Object.fromEntries(UNARY.map((op) => [op, ({ a }) => ok(isOperand(a) ? a[op]() : ip.ops[op](a))])),
  round: ({ a, ndigits }) => ok(isOperand(a) ? a.round(ndigits) : ip.ops.round(a, ndigits)),
  matmul: ({ n, die }) => ok(ip.Die.sum(n, die)),
  d: ({ sides }) => ok(ip.d(sides)),
  z: ({ sides }) => ok(ip.z(sides)),
  Again: () => ok(ip.Again),
  Reroll: () => ok(ip.Reroll),
  Restart: () => ok(ip.Restart),
  Ellipsis: () => ok(ip.Ellipsis),
  die: ({ outcomes }) => isList(outcomes) && ok(new ip.Die(outcomes)),
  die_typed: ({ outcomes }) => isList(outcomes) && ok(new ip.Die(outcomes)),
  die_vectors: ({ outcomes }) => isList(outcomes) && ok(new ip.Die(outcomes)),
  die_symbols: ({ outcomes }) => isList(outcomes) && ok(new ip.Die(outcomes)),
  die_times: ({ outcomes, times }) => isList(outcomes) && ok(new ip.Die(outcomes, times)),
  mixture: ({ outcomes, times }) => isList(outcomes) && ok(new ip.Die(outcomes, times)),
  again: ({ outcomes, depth, end }) => isList(outcomes) && ok(new ip.Die(outcomes, { againDepth: depth, againEnd: end })),
  again_count: ({ outcomes, count, depth, end }) =>
    isList(outcomes) && count !== null && ok(new ip.Die(outcomes, { againCount: count, againDepth: depth, againEnd: end })),
  tuple: ({ items }) => isList(items) && ok(Array.from(items)),
  // The factories' own arity is checked by the core's catalogue (a TypeError).
  fn: ({ name, params }) =>
    name in FUNCTIONS && isList(params) && params.length === FUNCTIONS[name].length && ok(FUNCTIONS[name](...params)),
  evaluate: ({ name, params, args }) =>
    name in EVALUATORS && isList(params) && params.length === EVALUATORS[name].length && isList(args) && ok(EVALUATORS[name](...params).evaluate(...args)),
  multi_deal_evaluate: ({ name, params, args }) =>
    name in EVALUATORS && isList(params) && params.length === EVALUATORS[name].length && isList(args) && ok(EVALUATORS[name](...params).evaluate(...args)),
  evaluate_kw: ({ name, params, args, kwargs }) =>
    name in EVALUATORS && isList(params) && params.length === EVALUATORS[name].length && isList(args) && isList(kwargs) &&
    kwargs.every((kv) => Array.isArray(kv) && kv.length === 2 && typeof kv[0] === "string") &&
    ok(EVALUATORS[name](...params).evaluate(...args, Object.fromEntries(kwargs))),

  // Die methods.
  reroll: ({ die, which, star, depth }) => isDie(die) && ok(die.reroll(which, { star, depth })),
  filter: ({ die, which, star, depth }) => isDie(die) && ok(die.filter(which, { star, depth })),
  truncate: ({ die, min, max }) => isDie(die) && ok(die.truncate(min, max)),
  clip: ({ die, min, max }) => isDie(die) && ok(die.clip(min, max)),
  split: ({ die, which, star }) => isDie(die) && ok(die.split(which, { star })),
  explode: ({ die, which, star, depth, end }) => isDie(die) && ok(die.explode(which, { star, depth, end })),
  simplify: ({ die }) => isDie(die) && ok(die.simplify()),
  zero: ({ die }) => isDie(die) && ok(die.zero()),
  zero_outcome: ({ die }) => isDie(die) && ok(die.zeroOutcome()),
  sign: ({ die }) => isDie(die) && ok(die.sign()),
  die_cmp: ({ die, other }) => isDie(die) && ok(die.cmp(other)),
  is_in: ({ die, outcomes }) => isDie(die) && ok(die.isIn(outcomes)),
  die_count: ({ die, rolls, outcomes }) => isDie(die) && ok(die.count(rolls, outcomes)),
  if_else: ({ die, if_true, if_false, count, depth, end }) =>
    isDie(die) && ok(die.ifElse(if_true, if_false, { againCount: count, againDepth: depth, againEnd: end })),
  die_highest: ({ die, rolls, keep, drop }) => isDie(die) && ok(die.highest(rolls, keep, drop)),
  die_lowest: ({ die, rolls, keep, drop }) => isDie(die) && ok(die.lowest(rolls, keep, drop)),
  die_middle: ({ die, rolls, keep, tie }) => isDie(die) && ok(die.middle(rolls, keep, { tie })),
  die_pool: ({ die, rolls }) => isDie(die) && ok(die.pool(rolls)),
  pad_to_denominator: ({ die, denominator, outcome }) => isDie(die) && ok(die.padToDenominator(denominator, outcome)),
  multiply_to_denominator: ({ die, denominator }) => isDie(die) && ok(die.multiplyToDenominator(denominator)),
  quantities_op: ({ die, method, x }) => isDie(die) && typeof method === "string" && camel(method) in die && ok(die[camel(method)](x)),
  equals: ({ a, b, simplify }) => isDie(a) && ok(a.equals(b, { simplify })),
  time_to_sum: ({ die, target, max_time, dnf }) => isDie(die) && ok(die.timeToSum(target, max_time, dnf)),
  group_by: ({ die, key, star }) => isDie(die) && ok(Array.from(die.groupBy(key, { star }))),
  group_by_index: ({ die, index }) => isDie(die) && ok(Array.from(die.groupByIndex(index))),
  die_stat: ({ die, stat }) => isDie(die) && STATS.has(stat) && ok(die[camel(stat)]()),
  die_quantile: ({ die, method, args }) =>
    isDie(die) && typeof method === "string" && camel(method) in die && isList(args) && args.length >= 1 &&
    args.length <= 2 && ok(die[camel(method)](...args)),
  standardized_moment: ({ die, k }) => isDie(die) && ok(die.standardizedMoment(k)),
  entropy: ({ die, base }) => isDie(die) && isList(base) && base.length <= 1 && ok(die.entropy(...base)),
  cdf_distance: ({ a, b, stat }) => isDie(a) && typeof stat === "string" && camel(stat) in a && ok(a[camel(stat)](b)),
  joint_stat: ({ die, stat, i, j }) => isDie(die) && typeof stat === "string" && camel(stat) in die && ok(die[camel(stat)](i, j)),
  nearest: ({ die, comparison, outcome }) => isDie(die) && ok(die.nearest(comparison, outcome)),
  common_outcome_length: ({ die }) => isDie(die) && ok(die.commonOutcomeLength()),
  quantity: ({ die, args }) => isDie(die) && isList(args) && ok(die.quantity(...args)),
  quantities: ({ die, comparison }) => isDie(die) && ok(die.quantities(comparison)),
  probability: ({ die, args, percent }) => isDie(die) && isList(args) && ok(die.probability(...args, { percent })),
  probabilities: ({ die, comparison, percent }) => isDie(die) && ok(die.probabilities(comparison, { percent })),
  quantity_where: ({ die, which, star }) => isDie(die) && ok(die.quantityWhere(which, { star })),
  probability_where: ({ die, which, star, percent }) => isDie(die) && ok(die.probabilityWhere(which, { star, percent })),
  marginals: ({ die }) => isDie(die) && ok(die.marginals()),
  marginal: ({ die, index }) => isDie(die) && ok(die.marginal(index)),
  marginal_slice: ({ die, start, stop, step }) => isDie(die) && ok(die.marginalSlice(start, stop, step)),
  sample: ({ die, seed, n }) => isDie(die) && ok(die.sample({ seed, n })),
  format: ({ die, spec }) => isDie(die) && ok(spec === null ? die.toString() : die.format(spec)),
  die_repr: ({ die }) => isDie(die) && ok(die.repr()),
  die_method: ({ die, name, arg }) => {
    if (!isDie(die)) return undefined;
    switch (name) {
      case "getitem": return ok(die.get(arg));
      case "contains": return ok(die.has(arg));
      case "len": return ok(die.size);
      case "iter": return ok([...die]);
      case "keys": return ok(die.keys());
      case "values": return ok(die.values());
      case "items": return ok(die.items());
      case "min_outcome": case "max_outcome": case "is_empty": return ok(die[camel(name)]());
    }
    return undefined;
  },

  // Free functions.
  map: ({ f, args, star }) => isList(args) && ok(ip.map(f, ...args, { star })),
  map_kw: ({ f, args, star, kwargs }) =>
    isList(args) && isList(kwargs) && kwargs.length > 0 &&
    kwargs.every((kv) => Array.isArray(kv) && kv.length === 2 && typeof kv[0] === "string" && !["star", "repeat"].includes(kv[0])) &&
    new Set(kwargs.map((kv) => kv[0])).size === kwargs.length &&
    ok(ip.map(f, ...args, { star, ...Object.fromEntries(kwargs) })),
  map_repeat: ({ f, args, star, repeat }) => isList(args) && ok(ip.map(f, ...args, { star, repeat })),
  map_and_time: ({ f, initial, args, star, repeat }) => isList(args) && ok(ip.mapAndTime(f, initial, ...args, { star, repeat })),
  mean_time_to_absorb: ({ f, initial, args, star }) => isList(args) && ok(ip.meanTimeToAbsorb(f, initial, ...args, { star })),
  reduce: ({ f, dice, initial }) => ok(ip.reduce(f, dice, { initial })),
  accumulate: ({ f, dice, initial }) => ok(ip.accumulate(f, dice, { initial })),
  highest: ({ args, keep, drop, default: dflt }) => isList(args) && ok(ip.highest(...args, { keep, drop, default: dflt })),
  lowest: ({ args, keep, drop, default: dflt }) => isList(args) && ok(ip.lowest(...args, { keep, drop, default: dflt })),
  middle: ({ args, keep, tie, default: dflt }) => isList(args) && ok(ip.middle(...args, { keep, tie, default: dflt })),
  tupleize: ({ args }) => isList(args) && ok(ip.tupleize(...args)),
  vectorize: ({ args }) => isList(args) && ok(ip.vectorize(...args)),
  coin: ({ n, d, max_denominator }) => ok(ip.coin(n, d, { maxDenominator: max_denominator })),
  one_hot: ({ sides }) => ok(ip.oneHot(sides)),
  stochastic_round: ({ x, max_denominator }) => ok(ip.stochasticRound(x, { maxDenominator: max_denominator })),
  from_cumulative: ({ outcomes, cumulative, reverse }) => ok(ip.fromCumulative(outcomes, cumulative, { reverse })),
  pointwise: ({ args, max, spread }) => isList(args) && spread === true && ok(max === true ? ip.pointwiseMax(...args) : max === false ? ip.pointwiseMin(...args) : undefined),
  extreme_outcome: ({ args, max, spread }) =>
    isList(args) && spread === true && typeof max === "boolean" && ok(max ? ip.maxOutcome(...args) : ip.minOutcome(...args)),
  harmonize_denominators: ({ dice, weights }) => isList(dice) && ok(ip.harmonizeDenominators(dice, weights)),
  format_probability_inverse: ({ probability, args }) =>
    isList(args) && args.length === 1 && ok(ip.formatProbabilityInverse(probability, args[0])),

  // Pools and multiset expressions.
  pool: ({ dice, times }) => isList(dice) && ok(new ip.Pool(dice, times)),
  pool_highest: ({ pool, keep, drop }) => isKeep(pool) && ok(pool.highest(keep, drop)),
  pool_lowest: ({ pool, keep, drop }) => isKeep(pool) && ok(pool.lowest(keep, drop)),
  pool_middle: ({ pool, keep, tie }) => isKeep(pool) && ok(pool.middle(keep, { tie })),
  pool_keep: ({ pool, index }) => isKeep(pool) && ok(pool.keep(index)),
  pool_index: ({ pool, index }) => isKeep(pool) && ok(pool.at(index)),
  pool_slice: ({ pool, start, stop, step }) => isKeep(pool) && ok(pool.slice(start, stop, step)),
  pool_mul: ({ pool, n }) => isExpr(pool) && ok(pool.mul(n)),
  pool_neg: ({ pool }) => isExpr(pool) && ok(pool.neg()),
  pool_add: ({ a, b }) => isExpr(a) && ok(a.add(b)),
  pool_expand: ({ pool, order }) => isExpr(pool) && ok(pool.expand(order)),
  pool_all_counts: ({ pool, filter }) => isExpr(pool) && ok(pool.allCounts(filter)),
  pool_count_subset: ({ pool, divisor, empty_divisor }) => isExpr(pool) && ok(pool.countSubset(divisor, { emptyDivisor: empty_divisor })),
  pool_largest_straight_and_outcome: ({ pool, priority }) => isExpr(pool) && ok(pool.largestStraightAndOutcome(priority)),
  pool_isdisjoint: ({ pool, other }) => isExpr(pool) && ok(pool.isdisjoint(other)),
  pool_issubset: ({ pool, other }) => isExpr(pool) && ok(pool.issubset(other)),
  pool_issuperset: ({ pool, other }) => isExpr(pool) && ok(pool.issuperset(other)),
  pool_compare: ({ pool, other, cmp }) => {
    const m = { proper_subset: "lt", proper_superset: "gt", equal_set: "equalSet", not_equal_set: "notEqualSet" }[cmp];
    return isExpr(pool) && m !== undefined && ok(pool[m](other));
  },
  ...Object.fromEntries(
    ["sum", "size", "empty", "any", "product_of_counts", "highest_outcome_and_count", "largest_count",
      "largest_count_and_outcome", "largest_straight", "all_straights"].map((m) => [
      `pool_${m}`,
      ({ pool }) => isExpr(pool) && ok(pool[camel(m)]()),
    ]),
  ),
  mx_evaluate: ({ expr, method }) => isExpr(expr) && typeof method === "string" && MX_EVALUATIONS.has(method) && ok(expr[camel(method)]()),
  mx_binary: ({ a, o, b }) => {
    const m = { "+": "add", "-": "sub", "&": "and", "|": "or", "^": "xor" }[o];
    return isExpr(a) && m !== undefined && ok(a[m](b));
  },
  mx_counts: ({ expr, o, comparison, n }) => {
    if (!isExpr(expr)) return undefined;
    switch (o) {
      case "neg": case "pos": return ok(expr[o]());
      case "keep_counts": return ok(expr.keepCounts(comparison, n));
      case "mul": case "floordiv": case "mod": case "unique": case "multiply_counts": case "divide_counts":
      case "modulo_counts":
        return ok(expr[camel(o)](n));
    }
    return undefined;
  },
  mx_filter: ({ expr, invert, target }) =>
    isExpr(expr) && typeof invert === "boolean" && ok(invert ? expr.dropOutcomes(target) : expr.keepOutcomes(target)),
  mx_force_order: ({ expr, order }) => isExpr(expr) && ok(expr.forceOrder(order)),
  weightless: ({ generator }) => isExpr(generator) && ok(generator.weightless()),
  mx_max_pair: ({ expr, keep, comparison, other, priority }) =>
    isExpr(expr) && typeof keep === "boolean" &&
    ok(keep ? expr.maxPairKeep(comparison, other, priority) : expr.maxPairDrop(comparison, other, priority)),
  mx_sort_pair: ({ expr, method, comparison, other, order, extra }) =>
    isExpr(expr) && typeof method === "string" && camel(method) in expr && method.startsWith("sort_pair") &&
    ok(expr[camel(method)](comparison, other, order, extra)),
  mx_versus: ({ expr, all, comparison, other }) =>
    isExpr(expr) && typeof all === "boolean" && ok(all ? expr.versusAll(comparison, other) : expr.versusAny(comparison, other)),
  mx_method: ({ name, args, keep_negative_counts }) => {
    if (!isList(args) || args.length === 0 || !isExpr(args[0])) return undefined;
    const [self, ...others] = args;
    switch (name) {
      case "difference": return ok(self.difference(...others, { keepNegativeCounts: keep_negative_counts }));
      case "additive_union": case "intersection": case "union": return ok(self[camel(name)](...others));
    }
    return undefined;
  },
  mx_map_counts: ({ args, function: f }) => isList(args) && ok(ip.MultisetExpression.mapCounts(...args, { function: f })),
  mx_mixture: ({ inners }) => ok(ip.multisetMixture(inners)),
  mx_tuple_index: ({ expr, index }) => expr instanceof ip.MultiDeal && ok(expr.hand(index)),
  mx_tuple_slice: ({ expr, start, stop, step }) => expr instanceof ip.MultiDeal && ok(expr.slice(start, stop, step)),

  // Decks and deals.
  deck: ({ outcomes, times }) => isList(outcomes) && ok(new ip.Deck(outcomes, times)),
  deal: ({ deck, hand_size }) => deck instanceof ip.Deck && !Array.isArray(hand_size) && ok(deck.deal(hand_size)),
  multi_deal: ({ deck, hand_sizes }) => deck instanceof ip.Deck && isList(hand_sizes) && ok(deck.deal(hand_sizes)),
  deal_groups: ({ deck, hand_groups }) => deck instanceof ip.Deck && isList(hand_groups) && ok(deck.dealGroups(...hand_groups)),
  deck_map: ({ deck, repl, star }) => deck instanceof ip.Deck && ok(deck.map(repl, { star })),
  deck_op: ({ operator, a, b }) => {
    const m = { "+": "add", "-": "sub", "&": "and", "|": "or", "^": "xor" }[operator];
    return a instanceof ip.Deck && m !== undefined && ok(a[m](b));
  },
  deck_mul: ({ deck, operator, n }) => {
    const m = { "*": "mul", "//": "floordiv", "%": "mod" }[operator];
    return deck instanceof ip.Deck && m !== undefined && ok(deck[m](n));
  },
  deck_pad_to_denominator: ({ deck, denominator, outcome }) => deck instanceof ip.Deck && ok(deck.padToDenominator(denominator, outcome)),
  deck_sequence: ({ deck, deals }) => deck instanceof ip.Deck && ok(deck.sequence(deals)),
  deck_stat: ({ deck, stat }) => deck instanceof ip.Deck && typeof stat === "string" && ok(deck.stat(stat)),
  deck_method: ({ deck, name, arg }) => {
    if (!(deck instanceof ip.Deck)) return undefined;
    switch (name) {
      case "symmetric_difference": return ok(deck.symmetricDifference(arg));
      case "getitem": return ok(deck.get(arg));
      case "keys": return ok(deck.keys());
      case "values": return ok(deck.values());
      case "iter": return ok([...deck]);
    }
    return undefined;
  },
  deal_str: ({ deal }) => (deal instanceof ip.Deal || deal instanceof ip.MultiDeal) && ok([deal.repr(), deal.toString()]),

  // Vector and Symbols.
  coll_binary: ({ a, b, name }) =>
    (a instanceof ip.Vector || a instanceof ip.Symbols) && typeof name === "string" && name in a && ok(a[name](b)),
  coll_unary: ({ a, name, ndigits }) =>
    (a instanceof ip.Vector || a instanceof ip.Symbols) && typeof name === "string" && name in a &&
    ok(name === "round" ? a.round(ndigits) : a[name]()),
  coll_truth: ({ x }) => ok(ip.truth(x)),
  truth: ({ x }) => x instanceof ip.DieWithTruth && ok(x.truth()),
};

const STATS = new Set(["mean", "variance", "standard_deviation", "sd", "skewness", "excess_kurtosis", "mode",
  "modal_quantity", "median", "median_low", "median_high", "min_outcome", "max_outcome", "denominator", "is_empty"]);
const MX_EVALUATIONS = new Set(["sum", "size", "empty", "any", "product_of_counts", "highest_outcome_and_count",
  "largest_count", "largest_count_and_outcome", "largest_straight", "all_straights", "expand", "all_counts"]);

// --- Results ------------------------------------------------------------------------------------------------------

const SPECIAL_REPRS = new Map([
  [ip.Reroll, "RerollType: <RerollType.Reroll: 'Reroll'>"],
  [ip.Restart, "RerollType: <RerollType.Restart: 'Restart'>"],
  [ip.Ellipsis, "ellipsis: Ellipsis"],
]);

/** encode.py `enc` of a JS value. */
function wire(x) {
  if (x === null) return null;
  switch (typeof x) {
    case "number":
      return Number.isInteger(x) ? BigInt(x) : { float: ip.pyFloatRepr(x) };
    case "bigint":
      return x;
    case "string":
      return { str: x };
    case "boolean":
      return { bool: x };
  }
  if (x instanceof ip.List) throw new Error("a list result: the wire format has no list");
  if (Array.isArray(x)) return { tuple: x.map(wire) };
  if (x instanceof ip.Float) return { float: ip.pyFloatRepr(x.value) };
  if (x instanceof ip.Fraction) return { frac: [x.numerator, x.denominator] };
  if (x instanceof ip.Vector) {
    // encode.py iterates the vector: a VectorWithTruthOnly raises there.
    if (x.items === null) throw new ip.IcepoolError("ValueError", "a VectorWithTruthOnly has no elements");
    return { vector: x.items.map(wire) };
  }
  if (x instanceof ip.Symbols) {
    if (x.str === null) throw new ip.IcepoolError("TypeError", "str() of these Symbols raises");
    return { symbols: x.str };
  }
  if (x instanceof ip.Die) return { die: dieItems(x) };
  if (x instanceof ip.Handle) return parseExact(x.toWire()).value;
  if (SPECIAL_REPRS.has(x)) return { repr: SPECIAL_REPRS.get(x) };
  if (x instanceof ip.Opaque) return { repr: x.repr };
  if (x instanceof ip.Complex) return { repr: `complex: ${x}` };
  throw new Error(`no wire encoding for ${x}`);
}

const dieItems = (die) => die.items().map(([o, q]) => [wire(o), q]);

/** encode.py `result_of`. */
export function resultOf(x) {
  if (x instanceof ip.Die) return { ok: true, die: dieItems(x) };
  if (x instanceof ip.Handle) return parseExact(x.toWire());
  return { ok: true, value: wire(x) };
}

/** Runs one fixture: null if it matches, else why not. */
export function check(fixture) {
  let actual;
  try {
    actual = resultOf(evaluate(fixture.program));
  } catch (e) {
    if (e instanceof ip.IcepoolError && e.unimplemented) return `port: ${e.message}`;
    actual = { ok: false, error: e?.name ?? String(e), message: e?.message };
  }
  const expected = fixture.expected;
  if (!actual.ok || !expected.ok) {
    if (actual.ok === expected.ok) return null;
    return expected.ok ? `raised ${actual.error}: ${actual.message}` : `oracle raised ${expected.error}, JS returned`;
  }
  const key = "die" in expected ? "die" : "value";
  const [e, a] = [canon(expected[key]), canon(actual[key])];
  return e === a ? null : `${key}: expected ${e.slice(0, 300)}, got ${a.slice(0, 300)}`;
}

/** Every fixture whose ops are all ported (as tests/fixtures.rs selects them). */
export function* fixtures() {
  const status = JSON.parse(fs.readFileSync(path.join(ROOT, "harness/grammar/status.json"), "utf8"));
  const ported = new Set(Object.keys(status).filter((op) => status[op] === "ported"));
  const files = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".jsonl")) files.push(p);
    }
  };
  walk(path.join(ROOT, "fixtures"));
  for (const file of files) {
    const lines = fs.readFileSync(file, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].trim()) continue;
      const fixture = parseExact(lines[i]);
      const ops = new Set();
      const walkOps = (x) => {
        if (Array.isArray(x)) x.forEach(walkOps);
        else if (x !== null && typeof x === "object") {
          if (typeof x.op === "string") ops.add(x.op);
          Object.values(x).forEach(walkOps);
        }
      };
      walkOps(fixture.program);
      if ([...ops].every((op) => ported.has(op))) {
        yield { file: path.relative(ROOT, file), line: i + 1, ...fixture };
      }
    }
  }
}
