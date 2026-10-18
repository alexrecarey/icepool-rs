// Translates upstream tests/sample_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

test("test_die_sample", () => {
  const result = ip.d(6).sample();
  assert.ok(result >= 1);
  assert.ok(result <= 6);
});

// Skipped upstream: "sampling expressions not supported for now", "still need to figure out how to sample multiset
// tuples".
test.skip("test_deck_sample", () => {});
test.skip("test_pool_sample", () => {});
test.skip("test_deal_sample", () => {});
