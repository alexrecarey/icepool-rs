// Every golden fixture through the JS API (see fixtures.mjs): the JS facade must give the oracle's results.
//   node --test test/                     (after build.sh --dev)
//   node test/fixtures.test.mjs fixtures/s4    only the fixture files under that path, failures listed

import { test } from "node:test";
import assert from "node:assert/strict";
import { check, fixtures, usage } from "./fixtures.mjs";

const only = process.argv[2];

test("every fixture through the JS API", () => {
  const failures = [];
  let n = 0;
  for (const f of fixtures()) {
    if (only && !f.file.startsWith(only)) continue;
    n++;
    const why = check(f);
    if (why) failures.push(`${f.file}:${f.line}: ${why}`);
  }
  const top = (m) => [...m].sort((a, b) => b[1] - a[1]).map(([op, c]) => `${op} ${c}`).join(", ");
  console.log(`${n} fixtures, ${failures.length} failing`);
  console.log(`idiomatic methods: ${top(usage.idiomatic)}`);
  console.log(`generic call(): ${top(usage.generic)}`);
  assert.ok(n > 0, "no fixtures found");
  assert.deepEqual(failures.slice(0, 40), [], `${failures.length} fixtures failed`);
});
