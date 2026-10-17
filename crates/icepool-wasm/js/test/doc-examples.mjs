// The documentation's examples, so that they cannot rot: every fenced `ts` block in README.md, docs/*.md and the
// TSDoc comments of src/*.ts. doc-examples.test.mjs runs each as its own module against dist/; `build.sh --test`
// also type-checks them (`node test/doc-examples.mjs emit <dir>` writes them as a tsc project).
//
// An example is ordinary TypeScript. Without an import from "icepool-wasm" it gets one of the whole public API, so
// short TSDoc examples can leave it out. A line ending in `//=> <expected>` checks the value of its expression:
//
//     d(6).mean().toString(); //=> "7/2"
//     d(6).items()[0]; //=> [1, 1n]
//     new Die([]).mean(); //=> throws ZeroDivisionError
//
// `<expected>` is a JS expression compared with node's `assert.deepStrictEqual` (so `1` is not `1n`, and a Fraction
// equals only a Fraction); `throws <Name>` expects the expression to throw an error of that `name` (an
// `IcepoolError`'s name is the Python exception's).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { stripTypeScriptTypes } from "node:module";

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, "..");

/** Every fenced `ts` block of the documentation: `{ file, line, code }`, `file` relative to js/, `line` the 1-based
 * line of the block's first code line. */
export function collectExamples() {
  const examples = [];
  const markdown = ["README.md", ...fs.readdirSync(path.join(ROOT, "docs")).filter((f) => f.endsWith(".md")).map((f) => `docs/${f}`)];
  for (const file of markdown) examples.push(...fenced(file, fs.readFileSync(path.join(ROOT, file), "utf8").split("\n")));
  for (const f of fs.readdirSync(path.join(ROOT, "src")).filter((f) => f.endsWith(".ts")).sort()) {
    // In a TSDoc comment, each line is ` * text`: strip the prefix, keeping line numbers.
    const lines = fs.readFileSync(path.join(ROOT, "src", f), "utf8").split("\n");
    examples.push(...fenced(`src/${f}`, lines.map((l) => l.replace(/^\s*\* ?/, ""))));
  }
  return examples;
}

function fenced(file, lines) {
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    const open = lines[i].match(/^\s*```(\S*)\s*$/);
    if (!open) continue;
    const start = i + 1;
    for (i++; i < lines.length && !/^\s*```\s*$/.test(lines[i]); i++);
    if (open[1] === "ts") out.push({ file, line: start + 1, code: lines.slice(start, i).join("\n") });
  }
  return out;
}

const CHECK = /^(\s*)(.*?);?\s*\/\/=>\s*(.*?)\s*$/;

/** The example as an ES module (TypeScript, before type stripping): the API imported from `api`, each `//=>` line
 * a call of `__check` (which the runner provides as a global). */
export function exampleSource(example, api, names) {
  const lines = example.code.split("\n").map((line, i) => {
    const m = line.match(CHECK);
    if (!m) return line;
    const [, indent, expr, expected] = m;
    const where = JSON.stringify(`${example.file}:${example.line + i}`);
    if (/^(const|let|var|import|export|return)\b/.test(expr)) {
      throw new Error(`${JSON.parse(where)}: //=> checks an expression, not a statement: ${line.trim()}`);
    }
    const thrown = expected.match(/^throws\s+(\w+)$/);
    return thrown
      ? `${indent}__checkThrows(() => (${expr}), ${JSON.stringify(thrown[1])}, ${where});`
      : `${indent}__check((${expr}), (${expected}), ${where});`;
  });
  const imports = /from\s+["']icepool-wasm["']/.test(example.code)
    ? ""
    : `import { ${names.join(", ")} } from "icepool-wasm";\n`;
  return (imports + lines.join("\n")).replace(/(from\s+)["']icepool-wasm["']/g, `$1${JSON.stringify(api)}`);
}

/** The example as JavaScript, ready to import. */
export function exampleModule(example, api, names) {
  return stripTypeScriptTypes(exampleSource(example, api, names), { mode: "strip" });
}

/** The public API's names (what `import * from "icepool-wasm"` gives). */
export async function publicNames() {
  return Object.keys(await import(pathToFileURL(path.join(ROOT, "dist/node.js")).href)).sort();
}

// `node test/doc-examples.mjs emit <dir>`: every example as `<dir>/NNN.mts` importing "icepool-wasm", with a
// tsconfig.json mapping that to dist/index.d.ts, for `tsc -p <dir>`.
if (process.argv[1] === fileURLToPath(import.meta.url) && process.argv[2] === "emit") {
  const dir = path.resolve(process.argv[3]);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const names = await publicNames();
  const declare = "declare function __check(actual: unknown, expected: unknown, where: string): void;\n" +
    "declare function __checkThrows(f: () => unknown, name: string, where: string): void;\n";
  fs.writeFileSync(path.join(dir, "globals.d.ts"), declare);
  collectExamples().forEach((example, i) => {
    const header = `// ${example.file}:${example.line}\n`;
    fs.writeFileSync(path.join(dir, `${String(i).padStart(3, "0")}.mts`), header + exampleSource(example, "icepool-wasm", names));
  });
  const tsconfig = {
    compilerOptions: {
      target: "es2022",
      lib: ["es2022", "esnext.disposable", "dom"],
      module: "nodenext",
      moduleResolution: "nodenext",
      strict: true,
      noEmit: true,
      skipLibCheck: true,
      types: [],
      paths: { "icepool-wasm": [path.relative(dir, path.join(ROOT, "dist/index.d.ts"))] },
    },
    include: ["*.mts", "globals.d.ts"],
  };
  fs.writeFileSync(path.join(dir, "tsconfig.json"), JSON.stringify(tsconfig, null, 2) + "\n");
}
