// Runs every example of the documentation (README.md, docs/*.md, the TSDoc of src/*.ts; see doc-examples.mjs for
// the `//=>` convention), and checks that every public symbol is documented with an example.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT, collectExamples, exampleModule, publicNames } from "./doc-examples.mjs";

const API = pathToFileURL(path.join(ROOT, "dist/node.js")).href;
const names = await publicNames();

globalThis.__check = (actual, expected, where) => assert.deepStrictEqual(actual, expected, `${where}: ${show(actual)}`);
globalThis.__checkThrows = (f, name, where) =>
  assert.throws(f, (e) => e?.name === name || assert.fail(`${where}: expected ${name}, got ${e?.name}: ${e?.message}`));

function show(x) {
  try {
    return JSON.stringify(x, (_, v) => (typeof v === "bigint" ? `${v}n` : v));
  } catch {
    return String(x);
  }
}

const examples = collectExamples();

test("the documentation has examples", () => assert.ok(examples.length > 0));

for (const example of examples) {
  test(`example ${example.file}:${example.line}`, async () => {
    const js = exampleModule(example, API, names);
    await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);
  });
}

// --- Every public symbol is documented ---------------------------------------------------------------------------
//
// Read from the generated declarations (dist/*.d.ts keep the TSDoc; `@internal` members are stripped): every name
// index.ts exports, and every public method of an exported class, needs a doc comment with an `@example`; every other
// public member of an exported class or interface (fields, getters, constructors, an interface's methods) a doc
// comment. One comment per overload set is enough. TypeDoc's `validation.notDocumented` (`npm run docs`)
// says the same, but needs the devDependencies.

/** The top-level declarations of a .d.ts and the members of its classes and interfaces, each with its doc comment
 * (`null` if none). */
function declarations(file) {
  const lines = fs.readFileSync(file, "utf8").split("\n");
  const out = [];
  let comment = null;
  let owner = null; // the class or interface whose body we are in
  let ownerKind = null;
  let depth = 0; // brace depth inside a member's type (an options object type spans lines)
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*\/\*\*/.test(line)) {
      let text = line;
      while (!lines[i].includes("*/")) text += "\n" + lines[++i];
      comment = text;
      continue;
    }
    if (depth > 0) {
      depth += count(line, "{") - count(line, "}");
      continue;
    }
    const top = line.match(/^export (?:declare )?(?:abstract )?(class|function|const|let|type|interface|namespace) (\w+)/);
    if (top) {
      out.push({ name: top[2], member: null, method: false, comment });
      if ((top[1] === "class" || top[1] === "interface") && !line.trimEnd().endsWith("}")) [owner, ownerKind] = [top[2], top[1]];
    } else if (owner !== null && /^}/.test(line)) {
      owner = null;
    } else if (owner !== null && /^ {4}\S/.test(line)) {
      const m = line.match(/^ {4}(?:(?:static|readonly|get|set|abstract) )*(\[[^\]]+\]|[\w$]+)/);
      // A method of a class (not a getter, a field, the constructor or a `[Symbol.*]` method) needs an example.
      const method = ownerKind === "class" && /^ {4}(static )?[\w$]+[<(]/.test(line) && m?.[1] !== "constructor";
      if (m && !/^ {4}(private|protected) /.test(line)) out.push({ name: owner, member: m[1], method, comment });
      depth = Math.max(0, count(line, "{") - count(line, "}"));
    }
    if (line.trim() !== "") comment = null;
  }
  return out;
}

const count = (s, c) => s.split(c).length - 1;

test("every public symbol has TSDoc, and every exported symbol and public method an example", () => {
  const exported = new Set(names);
  // Type-only exports (`export { type X }`) are in index.ts, not in the module's runtime names.
  const index = fs.readFileSync(path.join(ROOT, "src/index.ts"), "utf8");
  for (const m of index.matchAll(/type (\w+)/g)) exported.add(m[1]);
  // Overloads, and a class merged with an interface of the same name: one comment per symbol is enough.
  const docs = new Map(); // symbol -> its comments, joined
  for (const f of fs.readdirSync(path.join(ROOT, "dist")).filter((f) => f.endsWith(".d.ts"))) {
    for (const d of declarations(path.join(ROOT, "dist", f))) {
      if (!exported.has(d.name)) continue;
      const key = d.member === null ? d.name : `${d.name}.${d.member}`;
      const [comment, method] = docs.get(key) ?? ["", false];
      docs.set(key, [comment + (d.comment ?? ""), method || d.method || d.member === null]);
    }
  }
  const undocumented = [];
  for (const [key, [comment, needsExample]] of docs) {
    if (comment === "") undocumented.push(key);
    else if (needsExample && !comment.includes("@example")) undocumented.push(`${key} (no @example)`);
  }
  assert.deepEqual(undocumented, [], `${undocumented.length} public symbols lack TSDoc`);
});
