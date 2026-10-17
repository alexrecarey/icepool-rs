// Translates upstream tests/break_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, pyEq } from "./upstream.mjs";

const { Break, d, map, coin, Die, Reroll, mapAndTime, meanTimeToAbsorb, Restart } = ip;

test("test_self_loop_types", () => {
  // Test three different types of self-loop.
  const repl = (x, y) => {
    if (y === 1) return new Break();
    else if (y === 2) return x;
    else return Reroll;
  };

  assert.ok(pyEq(mapAndTime(repl, 0, d(3), { repeat: 10 }).marginals()[1].mean(), 0));
});

test("test_self_loop_types_mean_type_to_absorb", () => {
  // Test four different types of self-loop.
  const repl = (x, y) => {
    if (y === 1) return new Break();
    else if (y === 2) return x;
    else if (y === 3) return new Break(x);
    else return Reroll;
  };

  assert.ok(pyEq(meanTimeToAbsorb(repl, 0, d(4)), 0));
});

test("test_break_value_repeat", () => {
  const repl = (x, y) => {
    if (x >= 6) return new Break(-1);
    return x + y;
  };

  const result = map(repl, 0, d(2), { repeat: 10 });
  assert.ok(pyEq(result.probability(-1), 1));
});

test("test_break_value_repeat_inf", () => {
  const repl = (x, y) => {
    if (x >= 6) return new Break(-1);
    return x + y;
  };

  const result = map(repl, 0, coin(1, 2), { repeat: Infinity });
  assert.ok(pyEq(result.probability(-1), 1));
});

test("test_initial_break_repeat_inf", () => {
  const repl = (x) => {
    if (x <= 0 || x >= 3) return new Break();
    return coin(1, 2).radd(x);
  };

  const result = d(6).sub(3).map(repl, { repeat: Infinity });
  assert.ok(result.eq(new Die([-2, -1, 0, 3, 3, 3])).truth());
});

test("test_initial_break_mean_time_to_absorb", () => {
  const repl = (x) => {
    if (x === 1) return new Break();
    return coin(1, 2).rsub(x);
  };

  const result = d(10).meanTimeToAbsorb(repl);
  assert.ok(pyEq(result, 9));
});

test("test_restart_break", () => {
  const repl = (x, y) => {
    if (x === 3) return new Break();
    else if (y === 1) return Restart;
    else if (y === 2) return new Break();
    else return x + 1;
  };

  const result = map(repl, 0, d(3), { repeat: Infinity });
  assert.ok(result.eq(new Die(new Map([[0, 9n], [1, 3n], [2, 1n], [3, 1n]]))).truth());
});

test("test_break_example", () => {
  const example = (total, newRoll) => {
    if (total >= 10) return new Break(); // same as Break(total)
    else return total + newRoll;
  };

  const result = map(example, 0, d(6), { repeat: 20 });
  assert.deepEqual(new Set(result.outcomes()), new Set([10, 11, 12, 13, 14, 15]));
});
