// Translates upstream icepool tests/neon_city_overdrive_test.py.
import { test } from "node:test";
import { ip, assertDieEquals } from "./upstream.mjs";

const { multisetFunction, Die } = ip;
const d6 = ip.d(6);

// See https://rpg.stackexchange.com/q/171498/72732
// for approaches by myself and others.

const expected = new Die(
  new Map([
    [0, 8848236],
    [1, 73856630],
    [2, 153052510],
    [3, 257554860],
    [4, 389406800],
    [5, 550664770],
    [6, 493136280],
    [7, 196950375],
    [8, 46377500],
    [9, 6431250],
    [10, 487500],
    [11, 15625],
  ]),
);

function finalScore(outcome, count) {
  if (count === 0) return 0;
  else if (outcome < 6) return outcome;
  else return count + 5;
}

test("test_using_multiset_ops", () => {
  const midResult = d6.pool(6).sub(d6.pool(6)).highestOutcomeAndCount();

  const result = midResult.map(finalScore);
  assertDieEquals(result, expected);
});

test("test_using_multiset_function", () => {
  const midResult = multisetFunction((a, b) => a.sub(b).highestOutcomeAndCount());

  const result = midResult.call(d6.pool(6), d6.pool(6)).map(finalScore);
  assertDieEquals(result, expected);
});

test("test_keep_outcomes", () => {
  const usingKeep = d6.pool(6).keepOutcomes(d6.pool(6)).highestOutcomeAndCount();
  const usingMul = d6.pool(6).and(d6.pool(6).mul(100)).highestOutcomeAndCount();
  assertDieEquals(usingKeep, usingMul);
});

test("test_drop_outcomes", () => {
  const usingDrop = d6.pool(6).dropOutcomes(d6.pool(6)).highestOutcomeAndCount();
  const usingMul = d6.pool(6).sub(d6.pool(6).mul(100)).highestOutcomeAndCount();
  assertDieEquals(usingDrop, usingMul);
});

test("test_technoir", () => {
  const technoirDrop = multisetFunction((action, push, hurt) => {
    const survivingAction = action.dropOutcomes(hurt);
    const survivingPush = push.dropOutcomes(hurt);
    return [survivingAction.add(survivingPush).unique(2).highestOutcomeAndCount(), survivingPush.size()];
  });

  const technoirMul = multisetFunction((action, push, hurt) => {
    const survivingAction = action.sub(hurt.mul(100)).pos();
    const survivingPush = push.sub(hurt.mul(100)).pos();
    return [survivingAction.add(survivingPush).unique(2).highestOutcomeAndCount(), survivingPush.size()];
  });

  const usingDrop = technoirDrop.call(d6.pool(3), d6.pool(2), d6.pool(4));
  const usingMul = technoirMul.call(d6.pool(3), d6.pool(2), d6.pool(4));
  assertDieEquals(usingDrop, usingMul);
});
