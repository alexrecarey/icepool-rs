// Type-checks typical uses of the API against icepool.d.ts (build.sh --test runs `tsc --noEmit` on it; it is never
// executed). A declaration that drifts from core.js shows up here or in the JS tests.
import { d, Die, Pool, Deck, map, Again, Reroll, Break, Fraction, Order, MultisetEvaluator, evaluator, Ellipsis, ops } from "../icepool.js";
import type { Outcome } from "../icepool.js";

const d6: Die = d(6);
const sum: Die = d6.add(d6).sub(1).mul(2n);
const quantities: bigint[] = sum.quantities("<=");
const items: [Outcome, bigint][] = sum.items();
const mean: Fraction | number = d6.mean();
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
const product: unknown = ops.mul(3, 4);

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

export {
  quantities, items, mean, isTrue, rerolled, low, high, mapped, repeated, again, kept, atZero, expanded, hand, hands,
  quantity, product, counted, objectStates,
};
