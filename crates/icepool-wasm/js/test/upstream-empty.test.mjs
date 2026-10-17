// Translates upstream tests/empty_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const emptyDice = [
  ["Die([])", () => new ip.Die([])],
  ["Die([Reroll])", () => new ip.Die([ip.Reroll])],
  ["Die([Restart])", () => new ip.Die([ip.Restart])],
  ["Die({})", () => new ip.Die(new Map())],
];

for (const [name, make] of emptyDice) {
  test(`test_create_empty[${name}]`, () => {
    const result = make();
    assert.ok(result.isEmpty());
  });
}

test("test_op_empty", () => {
  const result = ip.d(6).add(new ip.Die([]));
  const expected = new ip.Die([]);
  assert.ok(result.equals(expected));
});

test("test_mix_empty", () => {
  const result = new ip.Die([ip.d(6), new ip.Die(new Map())]);
  const expected = ip.d(6);
  assert.ok(result.equals(expected));
});

test("test_map_empty", () => {
  const result = ip.map((x, y) => 0, new ip.Die(new Map()), new ip.Die(new Map()));
  const expected = new ip.Die(new Map());
  assert.ok(result.equals(expected));
});

test("test_zero_die_standard_pool", () => {
  assert.ok(ip.dPool([]).sum().eq(new ip.Die([0])).truth());
});
