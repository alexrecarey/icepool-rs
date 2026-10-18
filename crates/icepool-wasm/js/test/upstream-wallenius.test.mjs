// Translates upstream icepool tests/wallenius_test.py.
import { test } from "node:test";
import { ip, assertDieEquals } from "./upstream.mjs";

const { Wallenius, Die } = ip;

/** `lambda x: tuple(sorted(x))` for tuples of ints. */
const sorted = (x) => [...x].sort((a, b) => a - b);

test("test_wallenius_singleton_dict", () => {
  const data = new Map([[1, 1], [2, 2], [3, 3], [4, 4]]);

  const base = new Die(data);
  const expected = base.map((x) => base.remove(x).map((y) => [x, y])).map(sorted);

  assertDieEquals(new Wallenius(data).deal(2).expand(), expected);
});

test("test_wallenius_singleton_list", () => {
  const data = [[1, 1], [2, 2], [3, 3], [4, 4]];

  const base = new Die(new Map(data));
  const expected = base.map((x) => base.remove(x).map((y) => [x, y])).map(sorted);

  assertDieEquals(new Wallenius(data).deal(2).expand(), expected);
});

test("test_wallenius_weighted_dict", () => {
  const data = new Map([[1, [1, 2]], [2, [2, 2]], [3, [3, 2]], [4, [4, 2]]]);

  const base = new Die(new Map([1, 2, 3, 4].map((x) => [x, 2 * x])));
  const expected = base.map((x) => base.remove(x, x).map((y) => [x, y])).map(sorted);

  assertDieEquals(new Wallenius(data).deal(2).expand().simplify(), expected.simplify());
});

test("test_wallenius_weighted_list", () => {
  const data = [[1, 1], [2, 2], [3, 3], [4, 4], [1, 1], [2, 2], [3, 3], [4, 4]];

  const base = new Die(new Map([1, 2, 3, 4].map((x) => [x, 2 * x])));
  const expected = base.map((x) => base.remove(x, x).map((y) => [x, y])).map(sorted);

  assertDieEquals(new Wallenius(data).deal(2).expand().simplify(), expected.simplify());
});
