"""Task queue helper for loop.sh. PROTECTED.

Task definitions live in tasks.json (the agent may add tasks). Their status and attempt counts live in
loop/state.json, which only this script (via loop.sh, on main) writes: that keeps task branches and main from
conflicting.

  python3 loop/tasks.py next                 -> first todo task whose deps are all done; NONE (all done) / BLOCKED
  python3 loop/tasks.py field ID FIELD       -> a field of the task definition
  python3 loop/tasks.py status ID [STATUS]   -> get or set status (todo / done / blocked)
  python3 loop/tasks.py bump ID              -> attempts += 1, prints the new count
  python3 loop/tasks.py add-front ID TITLE DETAILS   -> insert a new task at the front (no deps)
  python3 loop/tasks.py merged               -> merges += 1, prints it
  python3 loop/tasks.py meta KEY [VALUE]     -> get or set a top-level state value
  python3 loop/tasks.py summary
"""
import json
import sys

TASKS, STATE = "tasks.json", "loop/state.json"


def load(p):
    with open(p) as f:
        return json.load(f)


def save(p, x):
    with open(p, "w") as f:
        json.dump(x, f, indent=2)
        f.write("\n")


def main(a):
    tasks, state = load(TASKS), load(STATE)
    st = state.setdefault("tasks", {})
    status = lambda tid: st.get(tid, {}).get("status", "todo")
    cmd = a[0]
    if cmd == "next":
        for t in tasks:
            if status(t["id"]) == "todo" and all(status(d) == "done" for d in t.get("deps", [])):
                print(t["id"]); return
        print("NONE" if all(status(t["id"]) == "done" for t in tasks) else "BLOCKED")
    elif cmd == "field":
        v = next(t for t in tasks if t["id"] == a[1]).get(a[2])
        print("" if v is None else (v if isinstance(v, str) else json.dumps(v)))
    elif cmd == "status":
        if len(a) == 2:
            print(status(a[1])); return
        st.setdefault(a[1], {})["status"] = a[2]; save(STATE, state)
    elif cmd == "bump":
        e = st.setdefault(a[1], {}); e["attempts"] = e.get("attempts", 0) + 1; save(STATE, state); print(e["attempts"])
    elif cmd == "add-front":
        if any(t["id"] == a[1] for t in tasks):
            return
        tasks.insert(0, {"id": a[1], "title": a[2], "deps": [], "milestone": None, "details": a[3]}); save(TASKS, tasks)
    elif cmd == "merged":
        state["merges"] = state.get("merges", 0) + 1; save(STATE, state); print(state["merges"])
    elif cmd == "meta":
        if len(a) == 2:
            v = state.get(a[1]); print("" if v is None else v); return
        state[a[1]] = a[2]; save(STATE, state)
    elif cmd == "summary":
        from collections import Counter
        c = Counter(status(t["id"]) for t in tasks)
        blocked = [t["id"] for t in tasks if status(t["id"]) == "blocked"]
        print(", ".join(f"{k} {v}" for k, v in sorted(c.items())) + (f"; blocked: {' '.join(blocked)}" if blocked else ""))


if __name__ == "__main__":
    main(sys.argv[1:])
