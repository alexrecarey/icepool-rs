// Translates upstream icepool tests/pair_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, assertDieEquals, pyEq } from "./upstream.mjs";

const { Pool, Die, Order, mapFunction, multisetFunction } = ip;
const [d4, d6, d8] = [ip.d(4), ip.d(6), ip.d(8)];

/** Python's `sorted(xs)`. */
const sorted = (xs) => [...xs].sort((a, b) => a - b);
/** Python's `zip(a, b)`. */
const zip = (a, b) => a.slice(0, Math.min(a.length, b.length)).map((x, i) => [x, b[i]]);

test("test_sort_pair_example_1", () => {
  const withoutHighest = new Pool([6, 4, 3]).sortPair(">", [5, 5]).expand();
  assertDieEquals(withoutHighest.simplify(), new Die([[6]]));
});

test("test_sort_pair_example_2", () => {
  const withHighest = new Pool([6, 4, 3]).sortPair(">", [5, 5], Order.Descending, "keep").expand().simplify();
  assertDieEquals(withHighest, new Die([[3, 6]]));
});

test("test_risk", () => {
  const result = d6.pool(3).highest(2).sortPair(">", d6.pool(2)).size();
  const expected = new Die(new Map([[0, 2275], [1, 2611], [2, 2890]]));
  assertDieEquals(result, expected);
});

test("test_risk_ascending", () => {
  const result = d6.pool(3).highest(2).sortPair(">", d6.pool(2), Order.Ascending).size();
  const expected = new Die(new Map([[0, 2275], [1, 2611], [2, 2890]]));
  assertDieEquals(result, expected);
});

const operators = {
  "==": (a, b) => a === b,
  "!=": (a, b) => a !== b,
  "<=": (a, b) => a <= b,
  "<": (a, b) => a < b,
  ">=": (a, b) => a >= b,
  ">": (a, b) => a > b,
};

const sortOps = ["==", "!=", "<=", "<", ">=", ">"];
const pyBool = (x) => (x ? "True" : "False");

for (const useDrop of [false, true]) {
  for (const op of sortOps) {
    test(`test_sort_pair_operators[${pyBool(useDrop)}-${op}]`, () => {
      let result;
      if (useDrop) {
        result = d6.pool(3).sortPair(op, d6.pool(2), Order.Descending, "drop").size();
      } else {
        result = d6.pool(3).highest(2).sortPair(op, d6.pool(2)).size();
      }

      const computeExpected = mapFunction((left, right) => {
        let result = 0;
        for (const [l, r] of zip([...left].reverse(), [...right].reverse())) {
          if (operators[op](l, r)) result += 1;
        }
        return result;
      });

      const expected = computeExpected(d6.pool(3), d6.pool(2));
      assertDieEquals(result, expected);
    });
  }
}

for (const useDrop of [false, true]) {
  for (const op of sortOps) {
    test(`test_sort_pair_operators_ascending[${pyBool(useDrop)}-${op}]`, () => {
      let result;
      if (useDrop) {
        result = d6.pool(3).sortPair(op, d6.pool(2), Order.Ascending, "drop").size();
      } else {
        result = d6.pool(3).lowest(2).sortPair(op, d6.pool(2), Order.Ascending).size();
      }

      const computeExpected = mapFunction((left, right) => {
        let result = 0;
        for (const [l, r] of zip(left, right)) {
          if (operators[op](l, r)) result += 1;
        }
        return result;
      });

      const expected = computeExpected(d6.pool(3), d6.pool(2));
      assertDieEquals(result, expected);
    });
  }
}

const sortPairLefts = [d6.pool(2), d6.pool(3), new Pool([d4, d6, d8])];
const sortPairRights = [d6.pool(2), new Pool([d4, d6])];

sortPairRights.forEach((right, ri) => {
  sortPairLefts.forEach((left, li) => {
    for (const op of sortOps) {
      test(`test_sort_pair_operators_expand[right${ri}-left${li}-${op}]`, () => {
        const result = left.highest(2).sortPair(op, right).expand();

        const computeExpected = mapFunction((left, right) => {
          const result = [];
          for (const [l, r] of zip([...left].reverse(), [...right].reverse())) {
            if (operators[op](l, r)) result.push(l);
          }
          return sorted(result);
        });

        const expected = computeExpected(left, right);
        assertDieEquals(result, expected);
      });
    }
  });
});

test("test_max_pair_example", () => {
  let result = new Pool([6, 4, 3, 1]).maxPairKeep("<=", [5, 5], "high").expand();
  assertDieEquals(result.simplify(), new Die([[3, 4]]));
  result = new Pool([6, 4, 3, 1]).maxPairDrop("<=", [5, 5], "high").expand();
  assertDieEquals(result.simplify(), new Die([[1, 6]]));
});

const maxPairOps = ["<=", "<", ">=", ">"];

for (const priority of ["low", "high"]) {
  for (const op of maxPairOps) {
    test(`test_max_pair_keep_size[${priority}-${op}]`, () => {
      const left = d6.pool(3);
      const right = d6.pool(2);
      const result = left.maxPairKeep(op, right, priority).size();
      const complement = left.maxPairDrop(op, right, priority).size();

      const computeExpected = mapFunction((left, right) => {
        if (["<=", "<"].includes(op)) {
          left = [...left].reverse();
          right = [...right].reverse();
        }
        left = [...left];
        right = [...right];
        let result = 0;
        while (left.length && right.length) {
          if (operators[op](left[0], right[0])) {
            result += 1;
            left.shift();
            right.shift();
          } else {
            left.shift();
          }
        }
        return result;
      });

      const expected = computeExpected(left, right);
      assertDieEquals(result, expected);
      assertDieEquals(result.rsub(3), complement);
    });
  }
}

const poolsToTest = [d6.pool(2), d6.pool(3), new Pool([d4, d6]), new Pool([d4, d6, d8])];

poolsToTest.forEach((right, ri) => {
  poolsToTest.forEach((left, li) => {
    for (const op of maxPairOps) {
      test(`test_max_pair_expand_late[right${ri}-left${li}-${op}]`, () => {
        const priority = ["<=", "<"].includes(op) ? "high" : "low";
        const result = left.maxPairKeep(op, right, priority).expand();

        const computeExpected = mapFunction((left, right) => {
          if (["<=", "<"].includes(op)) {
            left = [...left].reverse();
            right = [...right].reverse();
          }
          left = [...left];
          right = [...right];
          const result = [];
          while (left.length && right.length) {
            if (operators[op](left[0], right[0])) {
              result.push(left[0]);
              left.shift();
              right.shift();
            } else {
              left.shift();
            }
          }
          return sorted(result);
        });

        const expected = computeExpected(left, right);
        assertDieEquals(result, expected);
      });
    }
  });
});

poolsToTest.forEach((right, ri) => {
  poolsToTest.forEach((left, li) => {
    for (const op of maxPairOps) {
      test(`test_max_pair_expand_early[right${ri}-left${li}-${op}]`, () => {
        const priority = ["<=", "<"].includes(op) ? "low" : "high";
        const result = left.maxPairKeep(op, right, priority).expand();

        const computeExpected = mapFunction((left, right) => {
          if ([">=", ">"].includes(op)) {
            left = [...left].reverse();
            right = [...right].reverse();
          }
          left = [...left];
          right = [...right];
          for (let size = Math.min(left.length, right.length); size > 0; size--) {
            const leftSublist = left.slice(0, size);
            const rightSublist = right.slice(-size);
            if (zip(leftSublist, rightSublist).every(([l, r]) => operators[op](l, r))) {
              return sorted(leftSublist);
            }
          }
          return [];
        });

        const expected = computeExpected(left, right);
        assertDieEquals(result, expected);
      });
    }
  });
});

// The default priority should succeed even when pool sizes are not inferrable.
for (const op of maxPairOps) {
  test(`test_max_pair_default_priority[${op}]`, () => {
    const pool = d6.pool(3);
    pool.and(pool).maxPairKeep(op, pool.and(pool));
  });
}

const sorcerer = multisetFunction((win, lose) => {
  const winRemaining = win.sortPairDropWhile("==", lose);
  const loseRemaining = lose.sortPairDropWhile("==", win);
  return winRemaining.versusAll(">", loseRemaining).size();
});

const sorcererCases = [
  [[10, 4, 4, 1], [8, 8, 6, 5, 2, 2], 1],
  [[10, 8, 8, 4, 3], [2, 2, 2, 1, 1, 1], 5],
  [[8, 7, 6, 1], [8, 5, 5, 2], 2],
  [[10, 10, 1], [9, 7, 2, 2], 2],
  [[10, 6, 6, 4], [6, 5, 4, 3], 1],
  [[9, 9, 1, 1], [8, 8, 2, 2], 2],
  [[7, 6, 5, 5], [4, 3, 2, 1], 4],
  [[10, 10, 8, 7, 7, 5], [10, 5, 5, 5], 4],
  [[9, 9, 6, 6, 5], [9, 9, 6, 6, 4], 1],
  [[9, 8, 5, 5, 2], [8, 5, 5, 2, 1], 1],
];

sorcererCases.forEach(([win, lose, victories], i) => {
  test(`test_sorcerer[win${i}-lose${i}-${victories}]`, () => {
    assert.ok(pyEq(sorcerer.call(win, lose).probability(victories), 1));
  });
});

const donjon = multisetFunction((win, lose) => win.versusAll(">", lose.sortPairDropWhile("==", win)).expand());

const donjonCases = [
  [[6, 12, 15, 18], [4, 7, 9, 11, 12], [15, 18]],
  [[3, 11, 12, 13, 15], [5, 8, 10, 13, 15], [11, 12, 13, 15]],
];

donjonCases.forEach(([win, lose, victories], i) => {
  test(`test_donjon[win${i}-lose${i}-victories${i}]`, () => {
    assert.ok(pyEq(donjon.call(win, lose).probability(victories), 1));
  });
});
