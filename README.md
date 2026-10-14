# icepool-rs

A Rust port of [icepool](https://github.com/HighDiceRoller/icepool), aiming at full parity: a Rust core, the
same Python API on top (PyO3), and a JS API (WASM). Built by an unattended agent loop that checks every result
against Python icepool itself.

- What and why: [docs/ANALYSIS.md](docs/ANALYSIS.md)
- How the loop works: [docs/LOOP-PLAN.md](docs/LOOP-PLAN.md)
- Rules for agents: [AGENTS.md](AGENTS.md)
- Where it's at: [PROGRESS.md](PROGRESS.md), [tasks.json](tasks.json)
