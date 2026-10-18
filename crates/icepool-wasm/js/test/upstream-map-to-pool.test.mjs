// Translates upstream icepool tests/map_to_pool_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals } from "./upstream.mjs";

const { Die, tupleize, map } = ip;
const d6 = ip.d(6);

test("test_explode_to_pool", () => {
  assertDieEquals(d6.explodeToPool(3, null, { depth: 4 }).sum(), d6.explode(null, { depth: 4 }).rmatmul(3));
});

test("test_explode_to_pool_multi", () => {
  assertDieEquals(d6.explodeToPool(3, [5, 6], { depth: 4 }).sum(), d6.explode([5, 6], { depth: 4 }).rmatmul(3));
});

test("test_map_to_pool", () => {
  assertDieEquals(d6.pool(3).expand().mapToPool().expand(), d6.pool(3).expand());
});

test("test_reroll_to_pool_vs_reroll", () => {
  assertDieEquals(d6.rerollToPool(3, [1], { maxRerolls: 3 }).sum(), d6.reroll([1], { depth: 1 }).rmatmul(3));
});

test("test_reroll_pool_vs_map", () => {
  const expectedFunction = (rolls) => {
    if (rolls[2] < 4) {
      return new Die([
        tupleize(d6, rolls[1], rolls[2]),
        tupleize(rolls[0], d6, rolls[2]),
        tupleize(rolls[0], rolls[1], d6),
      ]);
    } else if (rolls[1] < 4) {
      return new Die([tupleize(d6, rolls[1], rolls[2]), tupleize(rolls[0], d6, rolls[2])]);
    } else if (rolls[0] < 4) {
      return tupleize(d6, rolls[1], rolls[2]);
    }
    return rolls;
  };
  const result = d6.rerollToPool(3, (x) => x < 4, { maxRerolls: 1 }).expand();
  const expected = map(expectedFunction, d6.pool(3)).map((rolls) => [...rolls].sort((a, b) => a - b));
  assertDieEquals(result.simplify(), expected.simplify());
  assert.equal(result.denominator(), 6n ** 4n);
});

test("test_reroll_pool_empowered_spell", () => {
  const result = d6.rerollToPool(8, [1, 2, 3], { maxRerolls: 4, mode: "low" }).sum();
  // `pytest.approx(33.61, abs=0.01)`.
  assert.ok(Math.abs(Number(result.mean()) - 33.61) <= 0.01, `${Number(result.mean())}`);
});

test("test_explode_empty", () => {
  const result = d6.explodeToPool(3, []);
  assertDieEquals(result.sum().simplify(), d6.rmatmul(3));
});

test("test_reroll_empty", () => {
  const result = d6.rerollToPool(3, [], { maxRerolls: 2 });
  assertDieEquals(result.sum().simplify(), d6.rmatmul(3));
});

test("test_reroll_drop", () => {
  const pool = d6.sub(3).rerollToPool(3, [0], { maxRerolls: 3, mode: "drop" });
  assert.equal(Number(pool.keepOutcomes([0]).size().mean()), 0);
});

test("test_reroll_drop_sum", () => {
  const a = d6.sub(3).rerollToPool(3, [0], { maxRerolls: 3, mode: "drop" }).sum();
  const b = d6.sub(3).rerollToPool(3, [0], { maxRerolls: 3, mode: "random" }).sum();
  assertDieEquals(a, b);
});
