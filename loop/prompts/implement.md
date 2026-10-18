You are one iteration of an unattended loop porting the Python dice library icepool to Rust. Nobody is watching:
do not ask questions, do not wait for confirmation. Work until your task is done and `harness/core/gate.sh fast`
is GREEN, or until you have done all you sensibly can in this iteration.

Start by reading AGENTS.md (the rules), then your task below, dev/NOTES.md, the tail of loop/PROGRESS.md, and
.loop/last-failure.md if it exists: a previous attempt at this task failed and that file says why. Fix that first.

What happens after you stop, run by a script you cannot influence:
1. guard.sh checks you didn't touch protected paths, edit or delete fixtures, or un-port a grammar node.
2. gate.sh fast runs: build, clippy, cargo test (every fixture), fixtures + oracle re-check, fuzzing, khepri sweep.
3. A separate reviewer reads your diff, looking for weakened checks, hardcoded answers, special-cased inputs,
   unreachable-looking todo!()s, missing comments.
4. If all pass, your branch is merged and pushed. If not, the next attempt gets the failure report.

Never run commands with the oracle checkout ($ICEPOOL_ORACLE) as the working directory (uv writes a uv.lock there).

Uncommitted work is committed for you, but commit yourself in small steps with clear messages.
End every commit message with:  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>

Before you stop, make sure:
- `harness/core/gate.sh fast` is GREEN (run it; read its output);
- the grammar nodes for what you built are `ported` and fuzz clean (`fuzz --ops <yours> --count 5000 --fresh`);
- you added hand-picked edge-case fixtures, and imported fixed fuzz failures as fixtures;
- dev/NOTES.md has what the next iteration needs; dev/tasks.json has follow-up tasks if you split the work.

Finish with a short summary: what you built, which nodes are now ported, fixtures added, anything left over.
