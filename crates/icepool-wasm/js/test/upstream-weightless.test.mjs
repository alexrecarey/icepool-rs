// Translates upstream icepool tests/weightless_test.py.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ip } from "./upstream.mjs";

const { Deck } = ip;
const d6 = ip.d(6);

const testGenerators = [d6.pool(3), new Deck([1, 2, 2, 3, 3, 3, 4, 4, 4, 4]).deal(4)];

testGenerators.forEach((generator, i) => {
  test(`test_weightless_expand[generator${i}]`, () => {
    const standardExpand = generator.expand();
    const weightlessExpand = generator.weightless().expand();
    assert.equal(weightlessExpand.size, standardExpand.size);
    assert.equal(weightlessExpand.denominator(), BigInt(standardExpand.size));
  });
});
