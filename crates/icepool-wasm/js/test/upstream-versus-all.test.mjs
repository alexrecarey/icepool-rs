// Translates upstream icepool tests/versus_all_test.py. (Upstream parametrizes over `order` without using it.)
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { Pool, Order } = ip;
const orders = [["Order.Ascending", Order.Ascending], ["Order.Descending", Order.Descending]];

for (const [name] of orders) {
  test(`test_versus_all_ge[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAll(">=", [1, 2]).expand().mode()[0], [2, 3]);
  });
}

for (const [name] of orders) {
  test(`test_versus_all_gt[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAll(">", [1, 2]).expand().mode()[0], [3]);
  });
}

for (const [name] of orders) {
  test(`test_versus_all_le[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAll("<=", [2, 3]).expand().mode()[0], [1, 2]);
  });
}

for (const [name] of orders) {
  test(`test_versus_all_lt[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAll("<", [2, 3]).expand().mode()[0], [1]);
  });
}

for (const [name] of orders) {
  test(`test_versus_any_ge[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAny(">=", [2, 3]).expand().mode()[0], [2, 3]);
  });
}

for (const [name] of orders) {
  test(`test_versus_any_gt[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAny(">", [2, 3]).expand().mode()[0], [3]);
  });
}

for (const [name] of orders) {
  test(`test_versus_any_le[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAny("<=", [1, 2]).expand().mode()[0], [1, 2]);
  });
}

for (const [name] of orders) {
  test(`test_versus_any_lt[${name}]`, () => {
    assert.deepEqual(new Pool([1, 2, 3]).versusAny("<", [1, 2]).expand().mode()[0], [1]);
  });
}
