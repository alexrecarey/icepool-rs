// Translates upstream tests/format_test.py.
import { test } from "node:test";
import { ip } from "./upstream.mjs";

const testDice = [new ip.Die([]), ip.d(6), ip.d(6).map((x) => [x, x + 1])];

// Test against errors only.

for (const die of testDice) {
  test(`test_repr[${die.repr()}]`, () => {
    die.repr();
  });
}

for (const die of testDice) {
  test(`test_str[${die.repr()}]`, () => {
    die.toString();
  });
}

const formatSpecs = [
  "",
  "md:q==|q<=|q>=",
  "md:p==|p<=|p>=",
  "md:%==|%>=|%>=",
  "csv:q==|q<=|q>=",
  "csv:p==|p<=|p>=",
  "csv:%==|%>=|%>=",
  "bbcode:q==|q<=|q>=",
  "bbcode:p==|p<=|p>=",
  "bbcode:%==|%>=|%>=",
  "html:q==|q<=|q>=",
  "html:p==|p<=|p>=",
  "html:%==|%>=|%>=",
];

for (const die of testDice) {
  for (const formatSpec of formatSpecs) {
    // Python's f'{die:{format_spec}}' is die.format(format_spec).
    test(`test_format_spec[${formatSpec}-${die.repr()}]`, () => {
      die.format(formatSpec);
    });
  }
}
