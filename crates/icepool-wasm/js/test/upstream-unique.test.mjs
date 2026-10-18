// Translates upstream icepool tests/unique_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { Pool, dPool } = ip;
const [d6, d8, d10] = [ip.d(6), ip.d(8), ip.d(10)];

const testPools = [
  dPool([4, 4]),
  dPool([6, 6, 6]),
  dPool([6, 6, 6, 6]).keep([0, 0, 0, 1]),
  dPool([6, 6, 6, 6]).keep([0, 1, 1, 1]),
  dPool([6, 6, 6, 6]).keep([-1, 0, 0, 1]),
  dPool([12, 10, 8, 6, 6, 4]),
  new Pool([d6.neg(), d8.neg(), d10.neg()]),
  d6.rmatmul(3).pool(6).slice(-3),
];

testPools.forEach((pool, i) => {
  test(`test_unique_vs_expand[pool${i}]`, (t) => {
    if (pool.keepTuple().some((x) => x < 0)) {
      t.skip();
      return;
    }
    const unique = (x) => [...new Set(x)].sort((a, b) => a - b);
    const result = pool.unique().expand();
    const expected = pool.expand().map(unique);
    assert.ok(result.equals(expected));
  });
});

testPools.forEach((pool, i) => {
  test(`test_unique_size[pool${i}]`, (t) => {
    if (pool.keepTuple().some((x) => x < 0)) {
      t.skip();
      return;
    }
    const a = pool.unique().expand().map((x) => x.length);
    const b = pool.unique().size();
    assert.ok(a.equals(b));
  });
});
