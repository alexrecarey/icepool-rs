// Translates upstream tests/symbols_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { Die, Symbols } = ip;
/** `Die([...]).map(Symbols)`. */
const symbolsDie = (strings) => new Die(strings).map((s) => new Symbols(s));
const die = symbolsDie(["", "a", "ab", "aab"]);

/** Python's `a == b` between dice, as a boolean. */
const dieEq = (a, b) => a.eq(b).truth();

test("test_marginals_getitem", () => {
  assert.ok(dieEq(die.marginal("a"), new Die([0, 1, 1, 2])));
  assert.ok(dieEq(die.marginal("b"), new Die([0, 0, 1, 1])));
  assert.ok(dieEq(die.marginal("ab"), die));
});

// `die.marginals.a`: attribute access on the marginals, which JS has no counterpart for (`marginal("a")` above).
test.skip("test_marginals_getattr", () => {});

test("test_add_new", () => {
  assert.ok(dieEq(die.add("c"), symbolsDie(["c", "ac", "abc", "aabc"])));
});

test("test_add_existing", () => {
  assert.ok(dieEq(die.add("b"), symbolsDie(["b", "ab", "abb", "aabb"])));
});

test("test_sub", () => {
  assert.ok(dieEq(die.sub("ac"), symbolsDie(["-ac", "-c", "b - c", "ab - c"])));
  assert.ok(dieEq(die.sub("ac").pos(), symbolsDie(["", "", "b", "ab"])));
});

test("test_and", () => {
  assert.ok(dieEq(die.and("abb"), symbolsDie(["", "a", "ab", "ab"])));
});

test("test_mul", () => {
  assert.ok(dieEq(die.mul(2), symbolsDie(["", "aa", "aabb", "aaaabb"])));
});

test("test_floordiv", () => {
  assert.ok(dieEq(die.floordiv(2), symbolsDie(["", "", "", "a"])));
});

test("test_issubset", () => {
  assert.ok(new Symbols("ab").issubset("abc"));
  assert.ok(!new Symbols("ab").issubset("ac"));
});

test("test_issuperset", () => {
  assert.ok(new Symbols("ab").issuperset("a"));
  assert.ok(!new Symbols("ab").issuperset("ac"));
});

test("test_ordering", () => {
  assert.ok(new Symbols("a").gt(new Symbols("")));
  assert.ok(new Symbols("a").gt(new Symbols("bb")));
  assert.ok(new Symbols("-a").lt(new Symbols("bb")));
  assert.ok(new Symbols("a").lt(new Symbols("ab")));
});

test("test_getitem_missing_does_not_add_symbol", () => {
  const symbols = new Symbols("a");
  assert.equal(symbols.get("b"), 0);
  assert.equal(symbols.method("getitem", "ab").repr(), new Symbols("a").repr());
  assert.deepEqual(symbols.keys(), ["a"]);
});

test("test_count_subset_missing_does_not_add_symbol", () => {
  const symbols = new Symbols("a");
  assert.equal(symbols.countSubset("b"), 0);
  assert.deepEqual(symbols.keys(), ["a"]);
});
