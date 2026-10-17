// Translates upstream tests/creation_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip, approx, raises } from "./upstream.mjs";

test("test_d_syntax", () => {
  assert.deepEqual(ip.d(6).quantities(), [1n, 1n, 1n, 1n, 1n, 1n]);
  assert.equal(Number(ip.d(6).probability(0)), 0.0);
  approx(ip.d(6).probability(1), 1.0 / 6.0);
  approx(ip.d(6).probability(6), 1.0 / 6.0);
  assert.equal(Number(ip.d(6).probability(7)), 0.0);
});

test("test_z", () => {
  assert.ok(ip.d(6).sub(1).eq(ip.z(6)).truth());
});

test("test_coin", () => {
  const b = ip.coin(1, 2);
  const c = ip.coin(1, 2);

  approx(b.probabilities(), [0.5, 0.5]);
  approx(c.probabilities(), [0.5, 0.5]);
});

test("test_list_no_min_outcome", () => {
  const result = new ip.Die([2, 3, 3, 4, 4, 4, 5, 5, 6]);
  const expected = ip.d(3).rmatmul(2);
  assert.ok(result.equals(expected));
});

test("test_d6s", () => {
  const d6 = ip.d(6);
  assert.ok(d6.equals(ip.d(6)));
  assert.ok(d6.equals(new ip.Die([d6])));
  assert.ok(d6.equals(new ip.Die([ip.d(3), ip.d(3).add(3)])));
  assert.ok(d6.equals(new ip.Die(new Map([[1, 1n], [2, 1n], [3, 1n], [4, 1n], [5, 1n], [6, 1n]]))));
  assert.ok(d6.equals(new ip.Die([1, 2, 3, 4, 5, 6])));
});

// Skipped upstream (and relies on `is` identity).
test.skip("test_return_self", () => {});

test("test_negative_weight_error", () => {
  raises(() => new ip.Die(new Map([[1, -1n]])), "ValueError");

  raises(() => new ip.Die([1], [-1]), "ValueError");
});

test("test_not_orderable_error", () => {
  raises(() => new ip.Die([null]), "TypeError");
});

test("test_implicit_mapping_error", () => {
  raises(() => ip.d(6).add(new Map()), "TypeError");
});

test("test_empty_tuple", () => {
  const result = new ip.Die([[]]);
  assert.ok(result.equals(result.add([])));
});

test("test_denominator", () => {
  const result = new ip.Die([ip.d(3), ip.d(4), ip.d(6)]);
  assert.equal(result.denominator(), 36n);
});
