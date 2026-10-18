// Type-checks typical uses of the API against its generated declarations (dist/*.d.ts; build.sh --test runs `tsc --noEmit`
// on it; it is never executed).
import { d, Die, Pool, Deck, map, Again, Reroll, Break, Fraction, Order, MultisetEvaluator, evaluator, Ellipsis, ops, multisetFunction, fromRv, Wallenius, mapToPool, List, tupleize, vectorize, Vector, AgainExpression } from "../dist/index.js";
import type { Outcome, MultisetExpression, PyFloat } from "../dist/index.js";

const d6: Die = d(6);
const sum: Die = d6.add(d6).sub(1).mul(2n);
const quantities: bigint[] = sum.quantities("<=");
const items: [Outcome, bigint][] = sum.items();
const mean: Fraction | PyFloat = d6.mean();
const sd: PyFloat = d6.sd();
const isTrue: boolean = d6.eq(d6).truth();
const rerolled: Die = d6.reroll([1], { depth: Infinity }).filter((x: number) => x > 2, { depth: 1 });
const [low, high]: [Die, Die] = d6.split((x: number) => x <= 3);
const mapped: Die = map((a: number, b: number) => (a > b ? Reroll : [a, b]), d6, d6);
const repeated: Die = map((t: number, r: number) => (t >= 10 ? new Break() : t + r), 0, d6, { repeat: Infinity });
const again: Die = new Die([1, 2, Again.add(3)], { againDepth: 2 });
const pool: Pool = new Pool([d6, d6, d(8)]).highest(2);
const kept: Die = pool.keep([Ellipsis, 1]).sum();
const atZero: Die = pool.at(0);
const expanded: Die = new Pool([d6, d6]).expand(Order.Descending);
const hand: Die = new Deck([1, 2, 3], 4).deal(3).sum();
const hands: Die = new Deck([1, 2, 3], 4).deal([2, 2]).hand(1).largestCount();
const quantity: bigint = d6.quantity(">=", 5);
const product: Outcome = ops.mul(3, 4);
const dieProduct: Die = ops.mul(3, d6);
const againProduct: AgainExpression = ops.mul(d6, Again);
const againSum: AgainExpression = d6.add(Again);
const noRolls: [] = d6.sequence(0);
const plainTuple: Outcome[] = tupleize(1, 2);
const plainVector: Vector = vectorize(1, 2);

class Count extends MultisetEvaluator {
  nextState(state: number | null, order: 1 | -1 | 0, outcome: number, count: number) {
    return (state ?? 0) + count;
  }
}
const counted: Die = new Count().evaluate(new Pool([d6, d6]), [1, 2, 3]);
const objectStates: Die = evaluator({
  nextState: (s: { n: number } | null, order, outcome, count: number) => ({ n: (s?.n ?? 0) + count }),
  finalOutcome: (s: { n: number }) => s.n,
}).evaluate([1, 2]);
const twoWay = multisetFunction((a: MultisetExpression, b: MultisetExpression) => [a.sub(b).size(), b.sub(a).size()]);
const traced: Die = twoWay.call(new Pool([d6, d6]), [1, 2]);
const countIn: Die = multisetFunction((a: MultisetExpression, { target }: { target?: number[] } = {}) =>
  a.keepOutcomes(target ?? []).size(), { kwonly: ["target"] }).call(new Pool([d6]), { target: [6] });
const fromJsRv: Die = fromRv({ cdf: (xs: Outcome[]) => xs.map(() => 0.5) }, [1, 2], 10, { scale: 2 });
const dealt: Die = new Wallenius(new Map<Outcome, number | [number, number]>([[1, 2], [2, [1, 3]]])).deal(2).sum();
const viaMap: Die = d6.map(new Map([[1, 6]])).reroll(new Map([[6, null]]), { depth: 1 });
const sequences: Die = d6.sequence(2).add(d6.keep(3, -1)).add(d6.keepSlice(3, 1));
const timeToTen: Fraction = d6.meanTimeToSum(10);
const rounded: Die = d6.truediv(4).stochasticRound({ maxDenominator: 10 });
const pools: Die = d6.mapToPool((x: number) => List.of(x, x)).sum().add(mapToPool((x: number) => List.of(x), d6).sum())
  .add(d6.explodeToPool(2, [6], { depth: 1 }).sum()).add(d6.rerollToPool(2, [1], { maxRerolls: 1 }).sum());
{
  using scratch = d(20).add(d6);
  scratch.mean();
}
d6.add(1).free();

export {
  quantities, items, mean, sd, dieProduct, againProduct, againSum, noRolls, plainTuple, plainVector, isTrue, rerolled, low, high, mapped, repeated, again, kept, atZero, expanded, hand, hands,
  quantity, product, counted, objectStates, traced, countIn, fromJsRv, dealt, viaMap, sequences, timeToTen, rounded,
  pools,
};
