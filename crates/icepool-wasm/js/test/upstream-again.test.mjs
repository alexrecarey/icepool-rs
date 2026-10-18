// Translates upstream tests/again_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { Again, Die, Reroll, d } = ip;
const d6 = d(6);

/** Python's `a == b` between dice, as a boolean. */
const dieEq = (a, b) => a.eq(b).truth();

// The _evaluate tests call a private method of AgainExpression, which the JS API does not expose.
test.skip("test_again_evaluate", () => {});
test.skip("test_again_evaluate_plus_6", () => {});
test.skip("test_again_evaluate_plus_d6", () => {});
test.skip("test_again_evaluate_d6_plus", () => {});
test.skip("test_again_evaluate_plus_again", () => {});

test("test_explode_d6_depth_0", () => {
  const die = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again)], { againDepth: 0 });
  assert.ok(dieEq(die, d6));
});

test("test_explode_d6_depth_1", () => {
  const die = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again)], { againDepth: 1 });
  const expected = new Die(new Map([
    [1, 6n], [2, 6n], [3, 6n], [4, 6n], [5, 6n],
    [7, 1n], [8, 1n], [9, 1n], [10, 1n], [11, 1n], [12, 1n],
  ]));
  assert.ok(dieEq(die, expected));
});

test("test_again_plus_again_depth_0", () => {
  const die = new Die([1, 2, 3, 4, 5, Again.add(Again)], { againDepth: 0, againEnd: 3 });
  assert.ok(dieEq(die, new Die([1, 2, 3, 4, 5, 6])));
});

test("test_again_plus_again_depth_1", () => {
  const die = new Die([1, 2, 3, 4, 5, Again.add(Again)], { againEnd: 3 });
  assert.ok(dieEq(die, new Die([1, 2, 3, 4, 5, Die.sum(2, d6)])));
});

test("test_again_reroll_depth_0", () => {
  const die = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again)], { againDepth: 0, againEnd: Reroll });
  assert.ok(dieEq(die, d(5)));
});

test("test_again_reroll_depth_1", () => {
  const die = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again)], { againDepth: 1, againEnd: Reroll });
  assert.ok(dieEq(die, d6.map(new Map([[6, d(5).radd(6)]]))));
});

test("test_again_infinity", () => {
  const die = new Die([1, 2, 3, 4, 5, Again], { againDepth: 0, againEnd: Infinity });
  assert.ok(dieEq(die, new Die([1, 2, 3, 4, 5, Infinity])));
});

test("test_owod", () => {
  const result = new Die(new Map([[0, 7n], [1, 2n], [Again.add(1), 1n]]), { againDepth: 3 });
  const subExpected = new Die(new Map([[-1, 7n], [0, 2n], [1, 1n]]));
  let expected = subExpected;
  expected = expected.map(new Map([[1, subExpected.radd(1)]]));
  expected = expected.map(new Map([[2, subExpected.radd(2)]]));
  expected = expected.map(new Map([[3, ip.lowest(subExpected, 0).radd(3)]]));
  expected = expected.add(1);
  assert.ok(dieEq(result, expected));
});

test("test_is_additive", () => {
  assert.ok(Again.isAdditive);
  assert.ok(Again.add(Again).isAdditive);
  assert.ok(Again.rmatmul(2).isAdditive);
  assert.ok(!Again.rmul(2).isAdditive);
  assert.ok(Again.rmatmul(d6).isAdditive);
  assert.ok(!Again.rmatmul(d6.sub(2)).isAdditive);
});

for (const n of [0, 1, 2]) {
  test(`test_again_count[${n}]`, () => {
    const count = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again)], { againCount: n });
    const depth = d6.explode(null, { depth: n });
    assert.ok(dieEq(count, depth));
  });
}

test("test_again_count_double_restart", () => {
  const result = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again).add(Again)], { againCount: 1, againEnd: ip.Restart });
  const expected = d(5);
  assert.ok(dieEq(result.simplify(), expected));
});

test("test_again_count_double_reroll", () => {
  const result = new Die([1, 2, 3, 4, 5, ip.ops.add(6, Again).add(Again)], { againCount: 0, againEnd: Reroll });
  const expected = d6.map(new Map([[6, Die.sum(2, d(5)).radd(6)]]));
  assert.ok(dieEq(result, expected));
});
