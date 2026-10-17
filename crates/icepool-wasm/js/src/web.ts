// The browser / bundler entry point: instantiates the wasm-pack `--target web` build (js/pkg-web, made by build.sh)
// with top-level await, so importers see a ready module.

import init, * as wasm from "../pkg-web/icepool_wasm.js";
import { bind, type WasmModule } from "./wasm.js";

await init();
bind(wasm as unknown as WasmModule);

export * from "./index.js";
