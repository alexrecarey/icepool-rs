#!/usr/bin/env python3
"""What changed in upstream icepool between two refs, and which files of the port to revisit.

Lists the files under src/icepool/ that changed between OLD and NEW in an upstream icepool checkout (by default
$ICEPOOL_ORACLE, else ~/code/alexrecarey/icepool-oracle), and for each prints its row of MODULE-MAP.md: the Rust
core, Python facade, PyO3, TypeScript and WASM files that port it. It also flags

  - upstream files at NEW that MODULE-MAP.md has no row for (new modules, renames), and
  - MODULE-MAP.md rows whose upstream file does not exist at NEW (deleted or renamed upstream).

Read-only: it only runs `git rev-parse`, `git diff` and `git ls-tree` in the upstream checkout, never fetches or
checks anything out. A ref the checkout does not have is an error: fetch it yourself first (the oracle checkout
has upstream's tags only up to v0.25.4; `git -C $ICEPOOL_ORACLE fetch --tags <upstream remote>` gets the rest).
Standard library only.

Usage:

    scripts/upstream_changes.py OLD NEW [--repo PATH] [--map PATH] [--stat]

Examples, with refs the oracle checkout has (33e7e650 is upstream's v2.2.2 commit; HEAD is the pinned oracle):

    scripts/upstream_changes.py 33e7e650 HEAD          # the 3 fixes and `any()` the oracle adds to v2.2.2
    scripts/upstream_changes.py v0.25.4 HEAD --stat    # a big jump, with renames and deleted modules
    scripts/upstream_changes.py v2.2.2 v2.3.0 --repo ~/src/icepool   # a newer upstream release, in another clone

Exit status: 0 when every changed upstream module has a row and every row's module exists at NEW, 1 when
something is flagged, 2 on a usage error or a missing ref.
"""

import argparse
import os
import re
import subprocess
import sys

UPSTREAM_DIR = "src/icepool/"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_MAP = os.path.join(ROOT, "MODULE-MAP.md")
# A table row whose first cell is a backticked .py path, e.g. "| `population/die.py` | `die.rs` | ...".
ROW = re.compile(r"^\|\s*`([^`]+\.py)`\s*\|")


def fail(message):
    """Stops with exit status 2: the script could not run, which is not the same as finding something."""
    print(f"upstream_changes: {message}", file=sys.stderr)
    sys.exit(2)


def git(repo, *args):
    """Runs a read-only git command in the upstream checkout and returns its stdout."""
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if done.returncode != 0:
        fail(f"git {' '.join(args)} failed in {repo}:\n{done.stderr.strip()}")
    return done.stdout


def resolve(repo, ref):
    """The commit a ref names, or a clear error if the checkout does not have it."""
    done = subprocess.run(["git", "-C", repo, "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"],
                          capture_output=True, text=True)
    if done.returncode != 0:
        tags = git(repo, "tag", "--sort=-v:refname").split()
        fail(f"{repo} has no ref {ref!r} (its newest tags: {' '.join(tags[:6]) or 'none'}). "
             f"Fetch it first; this script never fetches.")
    return done.stdout.strip()


def read_map(path):
    """MODULE-MAP.md's rows: {upstream path: {column name: cell}}, columns named by the table's header row."""
    header, rows = None, {}
    if not os.path.isfile(path):
        fail(f"no module map at {path}")
    with open(path, encoding="utf-8") as f:
        for line in f:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if header is None and cells and cells[0] == "Upstream":
                header = cells
            elif ROW.match(line):
                if header is None:
                    fail(f"{path}: a module row before the table's header row")
                rows[ROW.match(line)[1]] = dict(zip(header, cells))
    if not rows:
        fail(f"{path} has no module rows")
    return rows


def upstream_path(path):
    """A repository path relative to src/icepool/, or None outside it."""
    return path[len(UPSTREAM_DIR):] if path.startswith(UPSTREAM_DIR) else None


# Where each column's paths are, relative to the repository root.
COLUMN_DIRS = {
    "Rust core": "crates/icepool-core/src/",
    "PyO3": "crates/icepool-py/src/",
    "TypeScript": "crates/icepool-wasm/js/src/",
    "WASM exports": "crates/icepool-wasm/src/",
}


def print_row(name, row):
    """A module's row, one column per line, each with the directory its paths are in."""
    for column, cell in row.items():
        if column == "Upstream":
            continue
        if column == "Python facade":
            cell = f"python/icepool/{name} ({cell})"
        elif column in COLUMN_DIRS and cell != "—":
            cell = f"{cell}   [in {COLUMN_DIRS[column]}]"
        print(f"      {column + ':':14} {cell}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("old", help="the upstream ref the map was last checked against (tag or commit)")
    parser.add_argument("new", help="the upstream ref to compare with")
    parser.add_argument("--repo", default=os.path.expanduser(
        os.environ.get("ICEPOOL_ORACLE", "~/code/alexrecarey/icepool-oracle")),
        help="the upstream icepool checkout (default: $ICEPOOL_ORACLE)")
    parser.add_argument("--map", default=DEFAULT_MAP, help="the module map (default: MODULE-MAP.md at the root)")
    parser.add_argument("--stat", action="store_true", help="also print git's diffstat of src/icepool/")
    args = parser.parse_args()

    if not os.path.isdir(args.repo):
        fail(f"no upstream checkout at {args.repo} (set ICEPOOL_ORACLE or pass --repo)")
    old, new = resolve(args.repo, args.old), resolve(args.repo, args.new)
    rows = read_map(args.map)
    map_name = os.path.relpath(args.map, ROOT) if args.map.startswith(ROOT + os.sep) else args.map

    print(f"upstream {args.old} ({old[:8]}) -> {args.new} ({new[:8]}), in {args.repo}")
    if args.stat:
        print(git(args.repo, "diff", "--stat", old, new, "--", UPSTREAM_DIR).rstrip())
    print()

    # -M: a renamed module shows as one change (R100 old new) rather than a deletion and an addition.
    changes = [line.split("\t") for line in
               git(args.repo, "diff", "--name-status", "-M", old, new, "--", UPSTREAM_DIR).splitlines()]
    flagged = 0
    print(f"{len(changes)} changed file(s) under {UPSTREAM_DIR}:")
    for status, *paths in changes:
        names = [upstream_path(p) for p in paths]
        print(f"  {status:5} {' -> '.join(names)}")
        # The path at NEW is the last one (a rename or copy has two); a deletion has only the path at OLD.
        current = None if status.startswith("D") else names[-1]
        for name in dict.fromkeys(names):
            if not name.endswith(".py"):
                continue
            if name in rows:
                if len(names) > 1:
                    print(f"    {name}:")
                print_row(name, rows[name])
            elif name == current:
                print(f"    {name}: NOT IN THE MAP (a new module?): port it and add a row")
                flagged += 1

    at_new = {upstream_path(p) for p in git(args.repo, "ls-tree", "-r", "--name-only", new, "--",
                                            UPSTREAM_DIR).splitlines()}
    at_new = {p for p in at_new if p and p.endswith(".py")}
    missing = sorted(at_new - rows.keys())
    vanished = sorted(rows.keys() - at_new)
    if missing:
        print(f"\nUpstream modules at {args.new} with no row in {map_name}:")
        for name in missing:
            print(f"  {name}")
    if vanished:
        print(f"\nRows in {map_name} whose module is not in upstream at {args.new}:")
        for name in vanished:
            print(f"  {name}")
            print_row(name, rows[name])
    flagged += len(missing) + len(vanished)
    if flagged == 0:
        print(f"\nThe map covers every module of upstream at {args.new}.")
    sys.exit(1 if flagged else 0)


if __name__ == "__main__":
    main()
