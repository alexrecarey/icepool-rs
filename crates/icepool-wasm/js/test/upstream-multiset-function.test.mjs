// Translates upstream icepool tests/multiset_function_test.py.
import { test } from "node:test";
import { ip, assertDieEquals } from "./upstream.mjs";

const { multisetFunction, Die, Deck, tupleize } = ip;
const d6 = ip.d(6);

test("test_single_expression", () => {
  const evaluator = multisetFunction((a) => a.sum());

  assertDieEquals(evaluator.call(d6.pool(3)), d6.rmatmul(3));
});

test("test_body_generator", () => {
  const evaluator = multisetFunction((a) => a.add(d6.pool(2)).sum());

  assertDieEquals(evaluator.call(d6.pool(1)), d6.rmatmul(3));
});

test("test_var_args", () => {
  // Python's `sum(pools, start=[])`: the additive union of the pools.
  const evaluator = multisetFunction((...pools) => pools.reduce((acc, p) => acc.add(p)).sum(), { params: 0, rest: true });

  assertDieEquals(evaluator.call(d6.pool(1), d6.pool(1), d6.pool(1)), d6.rmatmul(3));
});

test("test_star_auto", () => {
  const evaluator = multisetFunction((a, b) => a.and(b).size());

  const evaluatorUnpack = multisetFunction((hands) => {
    const [a, b] = [hands.hand(0), hands.hand(1)];
    return a.and(b).size();
  });

  const deck = new Deck([0, 1, 2, 3, 4], 4);
  const deal = deck.deal([5, 5]);
  assertDieEquals(evaluator.call(deal, { star: true }), evaluatorUnpack.call(deal));
  assertDieEquals(evaluator.call(deal), evaluatorUnpack.call(deal));
});

test("test_kwargs", () => {
  const evaluator = multisetFunction((x, { target } = {}) => x.keepOutcomes([target]).size(), { kwonly: ["target"] });

  for (let i = 1; i < 7; i++) {
    assertDieEquals(evaluator.call(d6.pool(1), { target: i }), d6.eq(1));
  }
});

test("test_constant", () => {
  const evaluator = multisetFunction((x) => d6.pool(3).sum());

  assertDieEquals(evaluator.call(d6.pool(1)), d6.rmatmul(3), { simplify: true });
});

test("test_constant_in_joint", () => {
  const evaluator = multisetFunction((x) => [d6.pool(3).sum(), x.sum()]);

  const result = evaluator.call(d6.pool(2));
  const expected = tupleize(d6.rmatmul(3), d6.rmatmul(2));

  assertDieEquals(result, expected, { simplify: true });
});

test("test_nested", () => {
  const inner = multisetFunction((x) => x.sum());

  const outer = multisetFunction((x) => inner.call(x));

  assertDieEquals(outer.call(d6.pool(3)), d6.rmatmul(3));
});

test("test_nested_call_branch", () => {
  const inner = multisetFunction((x) => x.sum());

  const outer = multisetFunction((x) => [x.sum(), inner.call(x)]);

  assertDieEquals(outer.call(d6.pool(3)), d6.rmatmul(3).map((x) => [x, x]));
});

test("test_joint_mixture", () => {
  const evaluator = multisetFunction((x) => [x.sum(), x.size()]);

  const mixture = ip.multisetMixture([d6.pool(1), d6.pool(2)]);
  const result = evaluator.call(mixture);
  const expected = new Die([tupleize(d6, 1), tupleize(d6.rmatmul(2), 2)]);

  assertDieEquals(result, expected, { simplify: true });
});
