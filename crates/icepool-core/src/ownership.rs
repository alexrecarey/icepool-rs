//! The host objects a core value alone owns, for a host's cycle collector.
//!
//! A host object (a Python outcome, a user's multiset source) sits in the core behind an `Arc`, and core values
//! share their parts through `Arc`s: two dice made from one share their outcomes, and a die's caches (its popped
//! dice, its index) hold more clones of the same outcomes. Python's cycle collector asks each container which
//! objects it references (`tp_traverse`) and subtracts one per answer from those objects' reference counts; an
//! object whose count stays above zero is reachable from outside. So a container may only report a reference it
//! holds *alone*: if two Python objects both reported the one reference an `Arc` keeps, Python would subtract two
//! for one and could free an object that is still in use. Reporting too little is safe (the cycle just leaks, as
//! it did before this module); reporting too much is not.
//!
//! [`owned_hosts`] walks everything a root value reaches through `Arc`s and keeps the `Arc`s whose every strong
//! reference is held from inside the root's own part of the graph: the root owns an `Arc` when all its holders
//! are owned, starting from the root itself. The `Arc`s form a DAG (they are immutable once shared), so one pass in
//! topological order decides each node after all its holders. A clone held anywhere else (a running computation
//! on another thread, another Python object) leaves its strong count above what the walk found, so that `Arc` and
//! everything only it holds are not owned, and not reported.

use std::any::Any;
use std::sync::Arc;

use rustc_hash::FxHashMap;

/// Implemented by every core value that can hold host objects: lists its `Arc` children ([`Owners::arc`]) and
/// its host objects ([`Owners::host`]), each once per clone it holds. A field it leaves out only makes the walk
/// report less, which is safe (see the module comment); one listed twice would report too much.
pub trait Traverse {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>);
}

/// The graph of `Arc`s below a root, built by [`Traverse::traverse`].
pub struct Owners<'a> {
    nodes: Vec<Node<'a>>,
    /// Node index by `Arc` pointer: each `Arc` is one node however many clones of it are held.
    index: FxHashMap<*const (), usize>,
    /// The node whose children are being listed.
    current: usize,
}

struct Node<'a> {
    /// `Arc::strong_count` when the node was first reached (the root: 1, its owner).
    strong: usize,
    /// One entry per clone this node holds.
    children: Vec<usize>,
    /// For a host object, the object (for the host to downcast).
    host: Option<&'a dyn Any>,
}

impl<'a> Owners<'a> {
    /// A clone of `arc` held by the current node.
    pub fn arc<T: Traverse + ?Sized>(&mut self, arc: &'a Arc<T>) {
        self.arc_with(arc, |owners, x| x.traverse(owners));
    }

    /// A clone of `arc` held by the current node, whose contents `list` lists (for an `Arc` of a type that is not
    /// itself `Traverse`, such as a slice of pairs).
    pub fn arc_with<T: ?Sized>(&mut self, arc: &'a Arc<T>, list: impl FnOnce(&mut Self, &'a T)) {
        self.child(Arc::as_ptr(arc).cast(), Arc::strong_count(arc), |owners| {
            list(owners, arc)
        });
    }

    /// A clone of an `Arc` of a host object held by the current node: a leaf.
    pub fn host<T: ?Sized>(&mut self, arc: &'a Arc<T>, object: &'a dyn Any) {
        let i = self.child(Arc::as_ptr(arc).cast(), Arc::strong_count(arc), |_| {});
        self.nodes[i].host = Some(object);
    }

    /// Records an edge from the current node to the node of `ptr`, listing its own children the first time.
    fn child(&mut self, ptr: *const (), strong: usize, list: impl FnOnce(&mut Self)) -> usize {
        let i = match self.index.get(&ptr) {
            Some(&i) => i,
            None => {
                let i = self.nodes.len();
                self.nodes.push(Node {
                    strong,
                    children: Vec::new(),
                    host: None,
                });
                self.index.insert(ptr, i);
                let parent = std::mem::replace(&mut self.current, i);
                list(self);
                self.current = parent;
                i
            }
        };
        self.nodes[self.current].children.push(i);
        i
    }
}

/// The host objects `root` alone owns (see the module comment), each once.
pub fn owned_hosts<T: Traverse + ?Sized>(root: &T) -> Vec<&dyn Any> {
    let mut owners = Owners {
        nodes: vec![Node {
            strong: 1,
            children: Vec::new(),
            host: None,
        }],
        index: FxHashMap::default(),
        current: 0,
    };
    root.traverse(&mut owners);
    let nodes = owners.nodes;

    // Kahn's algorithm: a node is decided once every edge into it has been seen, and is owned if every one of its
    // strong references came from an owned node. A node on a cycle (which immutable `Arc`s cannot form) would
    // never be decided, and stays unowned.
    let mut pending = vec![0usize; nodes.len()];
    for node in &nodes {
        for &c in &node.children {
            pending[c] += 1;
        }
    }
    let mut from_owned = vec![0usize; nodes.len()];
    let mut owned = vec![false; nodes.len()];
    owned[0] = true;
    let mut ready = vec![0];
    while let Some(n) = ready.pop() {
        for &c in &nodes[n].children {
            if owned[n] {
                from_owned[c] += 1;
            }
            pending[c] -= 1;
            if pending[c] == 0 {
                owned[c] = from_owned[c] == nodes[c].strong;
                ready.push(c);
            }
        }
    }
    nodes
        .iter()
        .zip(owned)
        .filter_map(|(node, owned)| node.host.filter(|_| owned))
        .collect()
}

impl<T: Traverse> Traverse for [T] {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        for x in self {
            x.traverse(owners);
        }
    }
}

impl<T: Traverse> Traverse for Option<T> {
    fn traverse<'a>(&'a self, owners: &mut Owners<'a>) {
        if let Some(x) = self {
            x.traverse(owners);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{HostObject, HostRef};
    use crate::value::{BinOp, UnOp};
    use crate::{Die, DieArg, Quantity, Result, Value};

    #[derive(Debug)]
    struct Obj(u32);

    impl HostObject for Obj {
        fn py_hash(&self) -> i64 {
            self.0.into()
        }
        fn eq(&self, a: &Value, b: &Value) -> bool {
            obj(a) == obj(b)
        }
        fn compare(&self, op: BinOp, a: &Value, b: &Value) -> Result<bool> {
            Ok(match op {
                BinOp::Lt => obj(a) < obj(b),
                BinOp::Le => obj(a) <= obj(b),
                BinOp::Gt => obj(a) > obj(b),
                _ => obj(a) >= obj(b),
            })
        }
        fn binary(&self, _: BinOp, a: &Value, _: &Value) -> Result<Value> {
            Ok(a.clone())
        }
        fn unary(&self, _: UnOp) -> Result<Value> {
            unreachable!()
        }
        fn truthy(&self) -> Result<bool> {
            Ok(true)
        }
        fn repr(&self) -> String {
            format!("Obj({})", self.0)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn obj(v: &Value) -> Option<u32> {
        match v {
            Value::Host(h) => h.as_any().downcast_ref::<Obj>().map(|o| o.0),
            _ => None,
        }
    }

    fn host(n: u32) -> Value {
        Value::Host(HostRef::new(Obj(n)))
    }

    fn owned(die: &Die<Value>) -> Vec<u32> {
        let mut v: Vec<u32> = owned_hosts(die)
            .into_iter()
            .map(|a| a.downcast_ref::<Obj>().unwrap().0)
            .collect();
        v.sort();
        v
    }

    fn die(outcomes: Vec<Value>) -> Die<Value> {
        let n = outcomes.len();
        Die::new(
            outcomes.into_iter().map(DieArg::Outcome).collect(),
            vec![Quantity::from(1u8); n],
        )
        .unwrap()
    }

    #[test]
    fn a_die_alone_owns_its_host_outcomes_even_inside_tuples() {
        let d = die(vec![
            host(1),
            Value::Tuple(vec![Value::Int(2.into()), host(2)].into()),
        ]);
        assert_eq!(owned(&d), [1, 2]);
    }

    #[test]
    fn its_caches_are_its_own_too() {
        let d = die(vec![host(1), host(2), host(3)]);
        d.pop_min().unwrap();
        d.pop_max().unwrap();
        assert_eq!(d.position(&host(2)), Some(1));
        assert_eq!(owned(&d), [1, 2, 3]);
    }

    #[test]
    fn nothing_shared_with_another_live_value_is_reported() {
        let d = die(vec![host(1), host(2)]);
        let clone = d.clone();
        assert_eq!(owned(&d), Vec::<u32>::new());
        drop(clone);
        assert_eq!(owned(&d), [1, 2]);
        // A second die with one of the same outcome objects: that one is shared, the other is not.
        let Some(Value::Host(h)) = d.outcomes().next().cloned() else {
            unreachable!()
        };
        let other = die(vec![Value::Host(h), host(3)]);
        assert_eq!(owned(&d), [2]);
        assert_eq!(owned(&other), [3]);
        drop(other);
        assert_eq!(owned(&d), [1, 2]);
    }
}
