// Translates upstream tests/from_cumulative_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const d6 = ip.d(6);

test("test_from_cumulative", () => {
  const outcomes = [1, 2, 3, 4, 5, 6];
  const cumulative = outcomes.map((x) => d6.le(x));
  assert.ok(ip.fromCumulative(outcomes, cumulative).eq(d6).truth());
});

test("test_from_cumulative_reverse", () => {
  const outcomes = [1, 2, 3, 4, 5, 6];
  const cumulative = outcomes.map((x) => d6.ge(x));
  assert.ok(ip.fromCumulative(outcomes, cumulative, { reverse: true }).eq(d6).truth());
});

// Needs scipy.stats.norm as the random variable.
test.skip("test_from_rv_norm", () => {});
