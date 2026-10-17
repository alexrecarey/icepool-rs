// Translates upstream tests/arithmetic_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const testDice = [
  ["d6", ip.d(6)],
  ["d8", ip.d(8)],
  ["d10.explode(depth=2)", ip.d(10).explode(null, { depth: 2 })],
];
const ints = Array.from({ length: 10 }, (_, k) => k - 5); // range(-5, 5)

for (const [name, a] of testDice) {
  for (const i of ints) {
    test(`test_die_int_add[${name}-${i}]`, () => {
      const result = a.add(i);
      const expected = ip.map((x, y) => ip.ops.add(x, y), a, i);
      assert.ok(result.equals(expected));
    });
  }
}

for (const [name, a] of testDice) {
  for (const i of ints) {
    test(`test_int_die_add[${i}-${name}]`, () => {
      const result = a.radd(i);
      const expected = ip.map((x, y) => ip.ops.add(x, y), i, a);
      assert.ok(result.equals(expected));
    });
  }
}

for (const [nameA, a] of testDice) {
  for (const [nameB, b] of testDice) {
    test(`test_die_die_add[${nameA}-${nameB}]`, () => {
      const result = a.add(b);
      const expected = ip.map((x, y) => ip.ops.add(x, y), a, b);
      assert.ok(result.equals(expected));
    });
  }
}

for (const [name, a] of testDice) {
  for (const i of ints) {
    test(`test_die_int_sub[${name}-${i}]`, () => {
      const result = a.sub(i);
      const expected = ip.map((x, y) => ip.ops.sub(x, y), a, i);
      assert.ok(result.equals(expected));
    });
  }
}

for (const [name, a] of testDice) {
  for (const i of ints) {
    test(`test_int_die_sub[${i}-${name}]`, () => {
      const result = a.rsub(i);
      const expected = ip.map((x, y) => ip.ops.sub(x, y), i, a);
      assert.ok(result.equals(expected));
    });
  }
}

for (const [nameA, a] of testDice) {
  for (const [nameB, b] of testDice) {
    test(`test_die_die_sub[${nameA}-${nameB}]`, () => {
      const result = a.sub(b);
      const expected = ip.map((x, y) => ip.ops.sub(x, y), a, b);
      assert.ok(result.equals(expected));
    });
  }
}

test("test_clip", () => {
  let result = ip.d(6);
  result = result.clip(2, 5);
  const expected = new ip.Die([2, 2, 3, 4, 5, 5]);
  assert.ok(result.equals(expected));
});

test("test_clip_skip", () => {
  let result = ip.d(6).mul(2);
  result = result.clip(3, 9);
  const expected = new ip.Die([3, 4, 6, 8, 9, 9]);
  assert.ok(result.equals(expected));
});

test("test_clip_outside", () => {
  let result = ip.d(6);
  result = result.clip(0, 7);
  const expected = new ip.Die([1, 2, 3, 4, 5, 6]);
  assert.ok(result.equals(expected));
});

test("test_truncate", () => {
  let result = ip.d(6);
  result = result.truncate(2, 5);
  const expected = new ip.Die([2, 3, 4, 5]);
  assert.ok(result.equals(expected));
});

test("test_truncate_outside", () => {
  let result = ip.d(6);
  result = result.truncate(0, 7);
  const expected = new ip.Die([1, 2, 3, 4, 5, 6]);
  assert.ok(result.equals(expected));
});

test("test_truncate_skip", () => {
  let result = ip.d(6).mul(2);
  result = result.truncate(3, 9);
  const expected = new ip.Die([4, 6, 8]);
  assert.ok(result.equals(expected));
});

test("test_abs_positive", () => {
  const result = ip.d(6).abs();
  const expected = ip.d(6);
  assert.ok(result.equals(expected));
});

test("test_abs_negative", () => {
  const result = ip.d(6).neg().abs();
  const expected = ip.d(6);
  assert.ok(result.equals(expected));
});

test("test_abs_cross_zero", () => {
  const result = ip.d(6).sub(3).abs();
  const expected = new ip.Die([0, 1, 2, 3], [1, 2, 2, 1]);
  assert.ok(result.equals(expected));
});

test("test_abs_cross_zero_nonuniform", () => {
  const result = ip.d(6).add(ip.d(6)).sub(7).abs();
  const expected = new ip.Die([0, 1, 2, 3, 4, 5], [6, 10, 8, 6, 4, 2]);
  assert.ok(result.equals(expected));
});

test("test_mod", () => {
  const result = ip.d(10).mod(4);
  const expected = new ip.Die([0, 1, 2, 3], [2, 3, 3, 2]);
  assert.ok(result.equals(expected));
});

test("test_div", () => {
  const result = ip.d(10).floordiv(4);
  const expected = new ip.Die([0, 1, 2], [3, 4, 3]);
  assert.ok(result.equals(expected));
});

test("test_simplify", () => {
  const result = new ip.Die([0, 1, 2], [2, 4, 6]).simplify();
  const expected = new ip.Die([0, 1, 2], [1, 2, 3]);
  assert.ok(result.equals(expected));
});

test("test_matmul_int_die", () => {
  const result = ip.d(6).rmatmul(2);
  const expected = ip.d(6).add(ip.d(6));
  assert.ok(result.equals(expected));
});

test("test_matmul_die_die", () => {
  const result = ip.d(6).matmul(ip.d(6));
  const expected = ip.d(6).map((x) => ip.d(6).rmatmul(x));
  assert.ok(result.equals(expected));
});

test("test_matmul_coin_die", () => {
  const d6 = ip.d(6);
  const result = ip.coin(1, 2).matmul(d6);
  const expected = ip.coin(1, 2).map((x) => d6.rmatmul(x));
  assert.ok(result.equals(expected));
});

test("test_d", () => {
  const result = ip.d(3).matmul(ip.d(3));
  const expected = new ip.Die(new Map([
    [1, 9n],
    [2, 12n],
    [3, 16n],
    [4, 12n],
    [5, 12n],
    [6, 10n],
    [7, 6n],
    [8, 3n],
    [9, 1n],
  ]));
  assert.ok(result.equals(expected));
});

test("test_d_negative", () => {
  const result = ip.d(7).sub(4).matmul(ip.d(3));
  assert.ok(result.equals(result.neg()));
});
