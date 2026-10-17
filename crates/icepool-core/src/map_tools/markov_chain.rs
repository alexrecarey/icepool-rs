//! Port of map_tools/markov_chain.py: the absorption distribution and mean absorption time of an absorbing
//! Markov chain, by exact integer Gaussian elimination.
//!
//! Python finds the reachable states by popping a `set` (the frontier), so the order of its transient states,
//! and so of its matrix rows, depends on hashes. The results do not: the solution is exact, the absorption die is
//! simplified and the mean time is a `Fraction`. Two things could still depend on that order, and the port
//! refuses (`Unsupported`) rather than guess:
//! - which spelling of a state is kept, when equal states with different spellings (`1`, `True`, `1.0`) are
//!   reached on different paths: the first one found becomes the dict key, passed to the transition function and
//!   kept in the result;
//! - which exception is raised when several states raise (only the type would differ; the port raises the first
//!   it finds).
//!
//! The port explores breadth first (a FIFO frontier): every state at a finite distance is stepped eventually.
//! When the chain has infinitely many states, Python's set order may reach a state that raises while a depth-first
//! search would wander off forever (e.g. a walk unbounded below that raises at its upper end); breadth first, the
//! port raises whenever Python can.

use std::collections::VecDeque;

use dashu_base::UnsignedAbs;
use dashu_int::IBig;
use dashu_ratio::RBig;
use rustc_hash::FxHashMap;

use super::core_impl::{int, parts, Next, TransitionCache, BREAK, DEFAULT, RESTART};
use crate::error::Error;
use crate::{Die, DieArg, Quantity, Value};

/// A row of the matrix, as `SparseVector`: column -> non-zero entry. Columns are transient state indices, plus
/// `visit` (`SpecialValue.Visit`) for the starting vector.
type Row = FxHashMap<usize, IBig>;

/// Whether two equal values are spelled the same, i.e. are the same object as far as Python's results can tell
/// (`1` and `True` and `1.0` are equal but not interchangeable; neither are `0.0` and `-0.0`).
fn same_spelling(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Complex(xr, xi), Value::Complex(yr, yi)) => {
            xr.to_bits() == yr.to_bits() && xi.to_bits() == yi.to_bits()
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| same_spelling(a, b))
        }
        // Host objects: the host knows; equal ones of the same kind are taken to be interchangeable.
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// The spellings of the states met so far.
#[derive(Default)]
struct Spellings(FxHashMap<Value, Value>);

impl Spellings {
    fn check(&mut self, v: &Value) -> crate::Result<()> {
        match self.0.get(v) {
            Some(first) if !same_spelling(first, v) => Err(Error::Unported(
                "equal states spelled differently in an absorbing Markov chain (Python's answer depends on set order)"
                    .into(),
            )),
            Some(_) => Ok(()),
            None => {
                self.0.insert(v.clone(), v.clone());
                Ok(())
            }
        }
    }
}

/// `row * a - other * b` (`SparseVector.__mul__` / `__sub__`), zeros dropped.
fn combine(row: &Row, a: &IBig, other: &Row, b: &IBig) -> Row {
    let mut out: Row = row.iter().map(|(&k, v)| (k, v * a)).collect();
    for (&k, v) in other {
        let e = out.entry(k).or_default();
        *e -= v * b;
        if *e == IBig::ZERO {
            out.remove(&k);
        }
    }
    out
}

/// `SparseVector.simplify`: divides the row by the gcd of its entries.
fn simplify(row: &mut Row) {
    use dashu_base::Gcd;
    let mut g = dashu_int::UBig::ZERO;
    for v in row.values() {
        g = g.gcd(v.unsigned_abs());
    }
    if g > dashu_int::UBig::ONE {
        let g = IBig::from(g);
        for v in row.values_mut() {
            *v = &*v / &g;
        }
    }
}

/// One elimination step: `fundamental_solve[i] = fundamental_solve[i] * pivot_row[pivot] - pivot_row *
/// fundamental_solve[i][pivot]`, then simplify. Python also runs it when `fundamental_solve[i][pivot]` is zero,
/// which only scales the row (and the scale cancels: only ratios within a row are used), so that is skipped.
fn eliminate(rows: &mut [Row], i: usize, pivot_index: usize) {
    let Some(f) = rows[i].get(&pivot_index).cloned() else {
        return;
    };
    let p = rows[pivot_index][&pivot_index].clone();
    let mut row = combine(&rows[i], &p, &rows[pivot_index], &f);
    simplify(&mut row);
    rows[i] = row;
}

/// `absorbing_markov_chain_impl`: computes the absorption distribution of an absorbing Markov chain, as a die in
/// simplest form (zero-weight outcomes will not be preserved), and the mean absorption time (`None` if a
/// `Restart` was possible).
///
/// `initial_die` is `Die([initial_state])`. Any state that leads only to itself is considered absorbing.
pub fn absorbing_markov_chain<F, E>(
    transition_cache: &mut TransitionCache<F>,
    initial_die: &Die<Value>,
) -> std::result::Result<(Die<Value>, Option<RBig>), E>
where
    F: FnMut(&Value, usize) -> std::result::Result<Next, E>,
    E: From<Error>,
{
    // Find all reachable states.
    let initial = transition_cache.self_loop_die(initial_die)?;
    // `self_loop_die(initial_die).group_by[0]`, then `[DEFAULT].marginals[1]` and `[BREAK].marginals[1]`.
    let group = |t: i64| -> crate::Result<Option<Die<Value>>> {
        let items: Vec<_> = initial
            .items()
            .iter()
            .filter(|(o, _)| parts(o)[0] == int(t))
            .map(|(o, q)| (parts(o)[1].clone(), q.clone()))
            .collect();
        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Die::from_dict(items)?))
        }
    };
    let Some(initial_transient) = group(DEFAULT)? else {
        // No transients; everything is absorbed immediately.
        return Ok((initial_die.simplify()?, Some(RBig::ZERO)));
    };
    let initial_absorb = match group(BREAK)? {
        Some(die) => die,
        None => Die::from_dict(vec![])?,
    };

    // state -> its index in `states` / `transitions`, in the order the states were stepped.
    let mut spellings = Spellings::default();
    let mut index: FxHashMap<Value, usize> = FxHashMap::default();
    let mut states: Vec<Value> = Vec::new();
    // The die representing the next distribution of each transient state; its outcomes are `(transition_type,
    // state)`.
    let mut transitions: Vec<Die<Value>> = Vec::new();
    let mut frontier: VecDeque<Value> = initial_transient
        .items()
        .iter()
        .map(|(o, _)| o.clone())
        .collect();
    let mut in_frontier: FxHashMap<Value, ()> = frontier.iter().map(|s| (s.clone(), ())).collect();
    for s in &frontier {
        spellings.check(s)?;
    }
    while let Some(curr_state) = frontier.pop_front() {
        crate::interrupt::poll()?;
        in_frontier.remove(&curr_state);
        let transition = transition_cache.step_state(&curr_state)?;
        index.insert(curr_state.clone(), states.len());
        states.push(curr_state);
        for (outcome, _) in transition.items() {
            let p = parts(outcome);
            if p[0] == int(RESTART) {
                continue;
            }
            spellings.check(&p[1])?;
            if p[0] == int(DEFAULT)
                && !index.contains_key(&p[1])
                && !in_frontier.contains_key(&p[1])
            {
                in_frontier.insert(p[1].clone(), ());
                frontier.push_back(p[1].clone());
            }
        }
        transitions.push(transition);
    }

    // Create the transient matrix to be solved.
    let t = states.len();
    let visit = t;
    // Supposing the transition matrix has the form
    //
    //     Q 0
    //     A I
    //
    // with the state vector on the right, then `fundamental_solve` is solving the equation `(I - Q) v = s`, where
    // `s` is the starting state vector, and `v` is the visit vector which gives the number of visits to each
    // transient state. We solve this in unnormalized form, so instead of being 1, `I` is equal to the denominator
    // of the transition from the source state.
    //
    // [dst_index][src]
    let mut fundamental_solve: Vec<Row> = vec![Row::default(); t];
    // [src_index][absorbing state]
    let mut absorption_matrix: Vec<Vec<(Value, Quantity)>> = vec![Vec::new(); t];
    let mut has_restart = false;
    let add = |row: &mut Row, col: usize, x: IBig| {
        let e = row.entry(col).or_default();
        *e += x;
        if *e == IBig::ZERO {
            row.remove(&col);
        }
    };
    for (src_index, transition) in transitions.iter().enumerate() {
        // The identity term.
        add(
            &mut fundamental_solve[src_index],
            src_index,
            IBig::from(transition.denominator()),
        );
        for (outcome, quantity) in transition.items() {
            let p = parts(outcome);
            if p[0] == int(BREAK) {
                if !quantity.is_zero() {
                    absorption_matrix[src_index].push((p[1].clone(), quantity.clone()));
                }
            } else if p[0] == int(RESTART) {
                // transition to nowhere
                has_restart = true;
            } else {
                // Minus Q.
                let dst_index = index[&p[1]];
                add(
                    &mut fundamental_solve[dst_index],
                    src_index,
                    -IBig::from(quantity.clone()),
                );
            }
        }
    }
    for (src_index, src) in states.iter().enumerate() {
        // Setting `s`.
        let q = initial_transient
            .items()
            .iter()
            .find(|(o, _)| o == src)
            .map_or(Quantity::ZERO, |(_, q)| q.clone());
        fundamental_solve[src_index].remove(&visit);
        if !q.is_zero() {
            fundamental_solve[src_index].insert(visit, IBig::from(q));
        }
    }

    // Solve the matrix using Gaussian elimination.

    // Put into upper triangular form.
    for pivot_index in 0..t {
        let Some(i) = (pivot_index..t).find(|&i| fundamental_solve[i].contains_key(&pivot_index))
        else {
            return Err(Error::InvalidArgument(
                "Matrix has deficient rank. This likely indicates that the Markov process has a chance of not terminating."
                    .into(),
            )
            .into());
        };
        fundamental_solve.swap(i, pivot_index);
        for i in pivot_index + 1..t {
            crate::interrupt::poll()?;
            eliminate(&mut fundamental_solve, i, pivot_index);
        }
    }

    // Solve for the visit vector `v`.
    for pivot_index in (0..t).rev() {
        for i in 0..pivot_index {
            crate::interrupt::poll()?;
            eliminate(&mut fundamental_solve, i, pivot_index);
        }
    }

    let mut mean_absorption_time = RBig::ZERO;
    // (n * absorption_row, d) per transient state with visits and absorptions.
    let mut results: Vec<(Vec<(Value, IBig)>, IBig)> = Vec::new();
    for (pivot_index, absorption_row) in absorption_matrix.iter().enumerate() {
        // n / d is an element of the visit vector `v`.
        let Some(n) = fundamental_solve[pivot_index].get(&visit) else {
            continue;
        };
        let d = &fundamental_solve[pivot_index][&pivot_index];

        // Compared to the normalized formula, I and Q were scaled up by a factor `transitions[pivot].denominator()`
        // so (I - Q)^-1 was reduced by the same factor. So we put the factor back here.
        mean_absorption_time += (RBig::from(n.clone()) / RBig::from(d.clone()))
            * RBig::from(IBig::from(transitions[pivot_index].denominator()));

        if !absorption_row.is_empty() {
            let row = absorption_row
                .iter()
                .map(|(o, q)| (o.clone(), n * IBig::from(q.clone())))
                .collect();
            results.push((row, d.clone()));
        }
    }

    let results_denominator = results.iter().fold(dashu_int::UBig::ONE, |acc, (_, d)| {
        crate::math::lcm(&acc, &d.unsigned_abs())
    });
    let results_denominator = IBig::from(results_denominator);
    // `defaultdict(int)`: insertion order, first key spelling kept.
    let mut normalized_index: FxHashMap<Value, usize> = FxHashMap::default();
    let mut normalized_results: Vec<(Value, IBig)> = Vec::new();
    for (row, d) in results {
        let factor = &results_denominator / &d;
        for (outcome, quantity) in row {
            let quantity = quantity * &factor;
            match normalized_index.get(&outcome) {
                Some(&i) => normalized_results[i].1 += quantity,
                None => {
                    normalized_index.insert(outcome.clone(), normalized_results.len());
                    normalized_results.push((outcome, quantity));
                }
            }
        }
    }
    let normalized_results = normalized_results
        .into_iter()
        .map(|(o, q)| {
            Quantity::try_from(q)
                .map(|q| (o, q))
                .map_err(|_| Error::InvalidArgument("negative weight".into()))
        })
        .collect::<crate::Result<Vec<_>>>()?;

    let transient_absorb = Die::from_dict(normalized_results)?.simplify()?;
    let result = Die::new(
        vec![
            DieArg::Die(initial_absorb.clone()),
            DieArg::Die(transient_absorb),
        ],
        vec![
            initial_absorb.denominator(),
            initial_transient.denominator(),
        ],
    )?
    .simplify()?;

    let mean_absorption_time = if has_restart {
        None
    } else {
        // The starting vector `s` was not normalized, so we divide it out here.
        Some(mean_absorption_time / RBig::from(IBig::from(initial_die.denominator())))
    };
    Ok((result, mean_absorption_time))
}
