// The node entry point: loads the wasm-pack `--target nodejs` build (js/pkg-node, made by build.sh) synchronously.
import { createRequire } from "node:module";
import { bind } from "./core.js";

bind(createRequire(import.meta.url)("./pkg-node/icepool_wasm.js"));

export * from "./core.js";
