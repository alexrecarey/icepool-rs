//! Python's `sorted()`: a port of CPython 3.13's `list.sort` (`Objects/listobject.c`, see its `listsort.txt`), an
//! adaptive, stable, natural mergesort (timsort, with the "powersort" merge strategy).
//!
//! Any correct stable sort gives the same result for a total order, but not for outcomes that are not totally
//! ordered: a float NaN compares false with everything, so where it lands depends on exactly which pairs the
//! algorithm compares. icepool sorts every die's outcomes with `sorted()`, so `Die([inf]) * 0` mixed with other
//! outcomes only orders like Python if the port compares the same pairs in the same order. That is also what makes
//! *whether* a sort raises (on an incomparable pair) match Python. Hence a faithful port rather than `sort_by`.
//!
//! Only `<` is ever used. The sort works on a permutation of indices; `lt` sees the elements.

use crate::error::Result;
use crate::Value;

/// When we get into galloping mode, we stay there until both runs win less often than MIN_GALLOP consecutive times.
const MIN_GALLOP: usize = 7;
/// Natural runs shorter than this are boosted artificially via binary insertion.
const MAX_MINRUN: usize = 64;

/// Sorts `items` as Python's `sorted()` would with `lt` as `<`, stopping at the first comparison that raises.
///
/// ```
/// use icepool_core::value::sort::timsort;
///
/// // Stable: pairs equal by their first element keep their order.
/// let items = vec![(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd')];
/// let sorted = timsort(items, |a, b| Ok(a.0 < b.0)).unwrap();
/// assert_eq!(sorted, [(1, 'b'), (1, 'd'), (2, 'a'), (2, 'c')]);
/// ```
pub fn timsort<T>(items: Vec<T>, lt: impl Fn(&T, &T) -> Result<bool>) -> Result<Vec<T>> {
    let n = items.len();
    let mut ms = MergeState {
        a: (0..n).collect(),
        lt: |i: usize, j: usize| lt(&items[i], &items[j]),
        min_gallop: MIN_GALLOP,
        pending: Vec::new(),
        listlen: n,
    };
    ms.sort()?;
    let order = ms.a;
    let mut slots: Vec<Option<T>> = items.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|i| slots[i].take().expect("each index once"))
        .collect())
}

/// `sorted(values)`.
///
/// ```
/// use icepool_core::value::sort::sorted;
/// use icepool_core::Value;
///
/// // `sorted([2, 1.5, True])` is `[True, 1.5, 2]`.
/// let s = sorted(vec![Value::from(2), Value::Float(1.5), Value::Bool(true)]).unwrap();
/// assert_eq!(s.iter().map(Value::to_string).collect::<Vec<_>>(), ["True", "1.5", "2"]);
/// // `sorted(['a', 1])` raises TypeError.
/// assert!(sorted(vec![Value::str("a"), Value::from(1)]).is_err());
/// ```
pub fn sorted(values: Vec<Value>) -> Result<Vec<Value>> {
    timsort(values, Value::py_lt)
}

/// `sorted(values, reverse=True)`: a reverse sort reverses, sorts stably and reverses back (so equal items keep
/// their order).
///
/// ```
/// use icepool_core::value::sort::sorted_reverse;
/// use icepool_core::Value;
///
/// // `sorted([1, 2.0, True, 2], reverse=True)` is `[2.0, 2, 1, True]`: equal items keep their order.
/// let items = vec![Value::from(1), Value::Float(2.0), Value::Bool(true), Value::from(2)];
/// let s = sorted_reverse(items).unwrap();
/// assert_eq!(s.iter().map(Value::to_string).collect::<Vec<_>>(), ["2.0", "2", "1", "True"]);
/// ```
pub fn sorted_reverse(mut values: Vec<Value>) -> Result<Vec<Value>> {
    values.reverse();
    let mut sorted = sorted(values)?;
    sorted.reverse();
    Ok(sorted)
}

/// A pending run: `a[base..base + len]` is sorted.
#[derive(Clone, Copy)]
struct Run {
    base: usize,
    len: usize,
    /// Depth in a conceptual binary merge tree (powersort); set when the next run is found.
    power: u32,
}

struct MergeState<F> {
    /// The permutation being sorted (indices into the caller's items).
    a: Vec<usize>,
    lt: F,
    /// This controls when we get *into* galloping mode. It's initialized to MIN_GALLOP. merge_lo and merge_hi tend
    /// to nudge it higher for random data, and lower for highly structured data.
    min_gallop: usize,
    /// A stack of pending runs yet to be merged.
    pending: Vec<Run>,
    listlen: usize,
}

/// How `merge_lo` / `merge_hi` finish (their `goto` labels).
enum End {
    Succeed,
    Copy,
}

/// Locate the proper position of key in a sorted slice; if the slice contains an element equal to key, return the
/// position immediately to the left of the leftmost equal element. `hint` is an index at which to begin the search,
/// `0 <= hint < a.len()`. The return value is the `k` in `0..=n` such that `a[k-1] < key <= a[k]`.
fn gallop_left(
    lt: &impl Fn(usize, usize) -> Result<bool>,
    key: usize,
    a: &[usize],
    hint: usize,
) -> Result<usize> {
    let n = a.len() as isize;
    let hint = hint as isize;
    let at = |i: isize| a[i as usize];
    let (mut lastofs, mut ofs): (isize, isize) = (0, 1);
    if lt(at(hint), key)? {
        // a[hint] < key -- gallop right, until a[hint + lastofs] < key <= a[hint + ofs].
        let maxofs = n - hint;
        while ofs < maxofs {
            if lt(at(hint + ofs), key)? {
                lastofs = ofs;
                ofs = (ofs << 1) + 1;
            } else {
                break;
            }
        }
        ofs = ofs.min(maxofs);
        // Translate back to offsets relative to &a[0].
        lastofs += hint;
        ofs += hint;
    } else {
        // key <= a[hint] -- gallop left, until a[hint - ofs] < key <= a[hint - lastofs].
        let maxofs = hint + 1;
        while ofs < maxofs {
            if lt(at(hint - ofs), key)? {
                break;
            }
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        // Translate back to positive offsets relative to &a[0].
        let k = lastofs;
        lastofs = hint - ofs;
        ofs = hint - k;
    }
    // Now a[lastofs] < key <= a[ofs], so key belongs somewhere to the right of lastofs but no farther right than
    // ofs. Do a binary search, with invariant a[lastofs-1] < key <= a[ofs].
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) >> 1);
        if lt(at(m), key)? {
            lastofs = m + 1; // a[m] < key
        } else {
            ofs = m; // key <= a[m]
        }
    }
    Ok(ofs as usize)
}

/// Exactly like `gallop_left`, except that if key already exists in the slice, finds the position immediately to
/// the right of the rightmost equal value: the `k` such that `a[k-1] <= key < a[k]`.
fn gallop_right(
    lt: &impl Fn(usize, usize) -> Result<bool>,
    key: usize,
    a: &[usize],
    hint: usize,
) -> Result<usize> {
    let n = a.len() as isize;
    let hint = hint as isize;
    let at = |i: isize| a[i as usize];
    let (mut lastofs, mut ofs): (isize, isize) = (0, 1);
    if lt(key, at(hint))? {
        // key < a[hint] -- gallop left, until a[hint - ofs] <= key < a[hint - lastofs].
        let maxofs = hint + 1;
        while ofs < maxofs {
            if lt(key, at(hint - ofs))? {
                lastofs = ofs;
                ofs = (ofs << 1) + 1;
            } else {
                break;
            }
        }
        ofs = ofs.min(maxofs);
        let k = lastofs;
        lastofs = hint - ofs;
        ofs = hint - k;
    } else {
        // a[hint] <= key -- gallop right, until a[hint + lastofs] <= key < a[hint + ofs].
        let maxofs = n - hint;
        while ofs < maxofs {
            if lt(key, at(hint + ofs))? {
                break;
            }
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        lastofs += hint;
        ofs += hint;
    }
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) >> 1);
        if lt(key, at(m))? {
            ofs = m; // key < a[m]
        } else {
            lastofs = m + 1; // a[m] <= key
        }
    }
    Ok(ofs as usize)
}

/// Compute a good value for the minimum run length; natural runs shorter than this are boosted artificially via
/// binary insertion. If n < MAX_MINRUN return n (it's too small to bother with fancy stuff). Else return a k,
/// MAX_MINRUN / 2 <= k <= MAX_MINRUN, such that n/k is close to, but strictly less than, an exact power of 2.
fn merge_compute_minrun(mut n: usize) -> usize {
    let mut r = 0; // becomes 1 if any 1 bits are shifted off
    while n >= MAX_MINRUN {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

/// Two adjacent runs begin at index s1. The first run has length n1, and the second run (starting at index s1+n1)
/// has length n2. The list has total length n. Compute the "power" of the first run.
fn powerloop(s1: usize, n1: usize, n2: usize, n: usize) -> u32 {
    let mut result = 0;
    // Midpoints a and b: a = s1 + n1/2, b = s1 + n1 + n2/2 = a + (n1 + n2)/2. Those may not be integers, so we
    // work with 2*a and 2*b instead. Emulate a/n and b/n one bit a time, until bits differ.
    let mut a = 2 * s1 + n1;
    let mut b = a + n1 + n2;
    loop {
        result += 1;
        if a >= n {
            // Both quotient bits are 1.
            a -= n;
            b -= n;
        } else if b >= n {
            // a/n bit is 0, b/n bit is 1.
            break;
        } // else both quotient bits are 0
        a <<= 1;
        b <<= 1;
    }
    result
}

impl<F: Fn(usize, usize) -> Result<bool>> MergeState<F> {
    fn islt(&self, x: usize, y: usize) -> Result<bool> {
        (self.lt)(x, y)
    }

    fn sort(&mut self) -> Result<()> {
        let mut nremaining = self.listlen;
        if nremaining < 2 {
            return Ok(());
        }
        // March over the array once, left to right, finding natural runs, and extending short natural runs to
        // minrun elements.
        let minrun = merge_compute_minrun(nremaining);
        let mut lo = 0;
        while nremaining > 0 {
            // Identify next run.
            let mut n = self.count_run(lo, nremaining)?;
            // If short, extend to min(minrun, nremaining).
            if n < minrun {
                let force = nremaining.min(minrun);
                self.binarysort(lo, force, n)?;
                n = force;
            }
            // Maybe merge pending runs, then push the new run on the stack.
            self.found_new_run(n)?;
            self.pending.push(Run {
                base: lo,
                len: n,
                power: 0,
            });
            lo += n;
            nremaining -= n;
        }
        self.merge_force_collapse()
    }

    /// Stable binary insertion sort of `a[lo..lo + n]`, where `a[lo..lo + ok]` is already sorted.
    fn binarysort(&mut self, lo: usize, n: usize, ok: usize) -> Result<()> {
        for ok in ok.max(1)..n {
            // Set l to where a[ok] belongs. Invariants: all a[..l] <= pivot, all a[r..ok] > pivot.
            let pivot = self.a[lo + ok];
            let (mut l, mut r) = (0, ok);
            while l < r {
                let m = (l + r) >> 1;
                if self.islt(pivot, self.a[lo + m])? {
                    r = m;
                } else {
                    l = m + 1;
                }
            }
            // Slide a[l..ok] to the right a slot to make room for the pivot.
            self.a.copy_within(lo + l..lo + ok, lo + l + 1);
            self.a[lo + l] = pivot;
        }
        Ok(())
    }

    /// The length of the run beginning at `lo`, spanning no more than `nremaining` elements. The run may be
    /// ascending or descending, but it is permuted in place, if needed, so that it's always ascending upon return.
    fn count_run(&mut self, lo: usize, nremaining: usize) -> Result<usize> {
        let next_smaller = |s: &Self, n: usize| s.islt(s.a[lo + n], s.a[lo + n - 1]);
        let next_larger = |s: &Self, n: usize| s.islt(s.a[lo + n - 1], s.a[lo + n]);
        // Try ascending run first.
        let mut n = 1;
        while n < nremaining {
            if next_smaller(self, n)? {
                break;
            }
            n += 1;
        }
        if n == nremaining {
            return Ok(n);
        }
        // lo[n] is strictly less. If n > 1, there are n elements in an ascending run terminated by the strictly
        // less lo[n]. If the first key < lo[n-1], *somewhere* along the way the sequence increased, so we're done
        // (there is no descending run). Else first key >= lo[n-1], which implies that the entire ascending run
        // consists of equal elements. In that case, this is a descending run, and we reverse the all-equal prefix.
        if n > 1 {
            if self.islt(self.a[lo], self.a[lo + n - 1])? {
                return Ok(n);
            }
            self.a[lo..lo + n].reverse();
        }
        n += 1; // in all cases it's been established that lo[n] has been resolved

        // Finish descending run. All-equal subruns are reversed in-place on the fly. Their original order will be
        // restored at the end by the whole-slice reversal.
        let mut neq = 0;
        // We're n elements into the slice, and the most recent neq+1 elements are all equal. This reverses them
        // in-place, and resets neq for reuse.
        let reverse_last_neq = |s: &mut Self, n: usize, neq: &mut usize| {
            if *neq > 0 {
                let k = *neq + 1;
                s.a[lo + n - k..lo + n].reverse();
                *neq = 0;
            }
        };
        while n < nremaining {
            if next_smaller(self, n)? {
                // This ends the most recent run of equal elements, but still in the "descending" direction.
                reverse_last_neq(self, n, &mut neq);
            } else if next_larger(self, n)? {
                // Descending run is over.
                break;
            } else {
                // Not x < y and not y < x implies x == y.
                neq += 1;
            }
            n += 1;
        }
        reverse_last_neq(self, n, &mut neq);
        self.a[lo..lo + n].reverse(); // transform to ascending run

        // And after reversing, it's possible this can be extended by a naturally increasing suffix; e.g.,
        // [3, 2, 3, 4, 1] makes an ascending run from the first 4 elements.
        while n < nremaining {
            if next_smaller(self, n)? {
                break;
            }
            n += 1;
        }
        Ok(n)
    }

    /// The next run has been identified, of length n2. If there's already a run on the stack, apply the
    /// "powersort" merge strategy: compute the topmost run's "power" (depth in a conceptual binary merge tree) and
    /// merge adjacent runs on the stack with greater power. The caller pushes the new run afterwards.
    fn found_new_run(&mut self, n2: usize) -> Result<()> {
        if let Some(top) = self.pending.last() {
            let power = powerloop(top.base, top.len, n2, self.listlen);
            while self.pending.len() > 1 && self.pending[self.pending.len() - 2].power > power {
                self.merge_at(self.pending.len() - 2)?;
            }
            self.pending.last_mut().expect("non-empty").power = power;
        }
        Ok(())
    }

    /// Regardless of invariants, merge all runs on the stack until only one remains.
    fn merge_force_collapse(&mut self) -> Result<()> {
        while self.pending.len() > 1 {
            let mut n = self.pending.len() - 2;
            if n > 0 && self.pending[n - 1].len < self.pending[n + 1].len {
                n -= 1;
            }
            self.merge_at(n)?;
        }
        Ok(())
    }

    /// Merge the two runs at stack indices i and i+1.
    fn merge_at(&mut self, i: usize) -> Result<()> {
        let Run {
            base: mut ssa,
            len: mut na,
            ..
        } = self.pending[i];
        let Run {
            base: ssb, len: nb, ..
        } = self.pending[i + 1];
        // Record the length of the combined runs; if i is the 3rd-last run now, also slide over the last run
        // (which isn't involved in this merge). The current run i+1 goes away in any case.
        self.pending[i].len = na + nb;
        if i + 3 == self.pending.len() {
            self.pending[i + 1] = self.pending[i + 2];
        }
        self.pending.pop();

        // Where does b start in a? Elements in a before that can be ignored (already in place).
        let k = gallop_right(&self.lt, self.a[ssb], &self.a[ssa..ssa + na], 0)?;
        ssa += k;
        na -= k;
        if na == 0 {
            return Ok(());
        }
        // Where does a end in b? Elements in b after that can be ignored (already in place).
        let nb = gallop_left(
            &self.lt,
            self.a[ssa + na - 1],
            &self.a[ssb..ssb + nb],
            nb - 1,
        )?;
        if nb == 0 {
            return Ok(());
        }
        // Merge what remains of the runs, using a temp array with min(na, nb) elements.
        if na <= nb {
            self.merge_lo(ssa, na, ssb, nb)
        } else {
            self.merge_hi(ssa, na, ssb, nb)
        }
    }

    /// Merge the na elements starting at ssa with the nb elements starting at ssb = ssa + na in a stable way,
    /// in-place. Must also have that a[ssa + na - 1] belongs at the end of the merge, and should have na <= nb.
    fn merge_lo(&mut self, ssa: usize, mut na: usize, ssb: usize, mut nb: usize) -> Result<()> {
        let tmp: Vec<usize> = self.a[ssa..ssa + na].to_vec();
        let (mut dest, mut pa, mut pb) = (ssa, 0, ssb);
        self.a[dest] = self.a[pb];
        dest += 1;
        pb += 1;
        nb -= 1;
        let end = 'merge: {
            if nb == 0 {
                break 'merge End::Succeed;
            }
            if na == 1 {
                break 'merge End::Copy;
            }
            let mut min_gallop = self.min_gallop;
            loop {
                let mut acount = 0; // # of times A won in a row
                let mut bcount = 0; // # of times B won in a row
                                    // Do the straightforward thing until (if ever) one run appears to win consistently.
                loop {
                    if self.islt(self.a[pb], tmp[pa])? {
                        self.a[dest] = self.a[pb];
                        dest += 1;
                        pb += 1;
                        bcount += 1;
                        acount = 0;
                        nb -= 1;
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                        if bcount >= min_gallop {
                            break;
                        }
                    } else {
                        self.a[dest] = tmp[pa];
                        dest += 1;
                        pa += 1;
                        acount += 1;
                        bcount = 0;
                        na -= 1;
                        if na == 1 {
                            break 'merge End::Copy;
                        }
                        if acount >= min_gallop {
                            break;
                        }
                    }
                }
                // One run is winning so consistently that galloping may be a huge win. So try that, and continue
                // galloping until (if ever) neither run appears to be winning consistently anymore.
                min_gallop += 1;
                loop {
                    min_gallop -= (min_gallop > 1) as usize;
                    self.min_gallop = min_gallop;
                    let k = gallop_right(&self.lt, self.a[pb], &tmp[pa..pa + na], 0)?;
                    acount = k;
                    if k > 0 {
                        self.a[dest..dest + k].copy_from_slice(&tmp[pa..pa + k]);
                        dest += k;
                        pa += k;
                        na -= k;
                        if na == 1 {
                            break 'merge End::Copy;
                        }
                        // na==0 is impossible now if the comparison function is consistent, but we can't assume
                        // that it is.
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    self.a[dest] = self.a[pb];
                    dest += 1;
                    pb += 1;
                    nb -= 1;
                    if nb == 0 {
                        break 'merge End::Succeed;
                    }
                    let k = gallop_left(&self.lt, tmp[pa], &self.a[pb..pb + nb], 0)?;
                    bcount = k;
                    if k > 0 {
                        self.a.copy_within(pb..pb + k, dest);
                        dest += k;
                        pb += k;
                        nb -= k;
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    self.a[dest] = tmp[pa];
                    dest += 1;
                    pa += 1;
                    na -= 1;
                    if na == 1 {
                        break 'merge End::Copy;
                    }
                    if acount < MIN_GALLOP && bcount < MIN_GALLOP {
                        break;
                    }
                }
                min_gallop += 1; // penalize it for leaving galloping mode
                self.min_gallop = min_gallop;
            }
        };
        match end {
            End::Succeed => self.a[dest..dest + na].copy_from_slice(&tmp[pa..pa + na]),
            End::Copy => {
                // The last element of run a belongs at the end of the merge.
                self.a.copy_within(pb..pb + nb, dest);
                self.a[dest + nb] = tmp[pa];
            }
        }
        Ok(())
    }

    /// Merge the na elements starting at ssa with the nb elements starting at ssb = ssa + na in a stable way,
    /// in-place, working from the right. Must also have that a[ssa + na - 1] belongs at the end of the merge, and
    /// should have na >= nb.
    fn merge_hi(&mut self, ssa: usize, mut na: usize, ssb: usize, mut nb: usize) -> Result<()> {
        let tmp: Vec<usize> = self.a[ssb..ssb + nb].to_vec();
        // Positions count down and may pass below 0 at the very end, hence isize.
        let at = |i: isize| i as usize;
        let mut dest = (ssb + nb - 1) as isize;
        let mut pa = (ssa + na - 1) as isize;
        let mut pb = nb as isize - 1;
        let basea = ssa;
        self.a[at(dest)] = self.a[at(pa)];
        dest -= 1;
        pa -= 1;
        na -= 1;
        let end = 'merge: {
            if na == 0 {
                break 'merge End::Succeed;
            }
            if nb == 1 {
                break 'merge End::Copy;
            }
            let mut min_gallop = self.min_gallop;
            loop {
                let mut acount = 0; // # of times A won in a row
                let mut bcount = 0; // # of times B won in a row
                                    // Do the straightforward thing until (if ever) one run appears to win consistently.
                loop {
                    if self.islt(tmp[at(pb)], self.a[at(pa)])? {
                        self.a[at(dest)] = self.a[at(pa)];
                        dest -= 1;
                        pa -= 1;
                        acount += 1;
                        bcount = 0;
                        na -= 1;
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                        if acount >= min_gallop {
                            break;
                        }
                    } else {
                        self.a[at(dest)] = tmp[at(pb)];
                        dest -= 1;
                        pb -= 1;
                        bcount += 1;
                        acount = 0;
                        nb -= 1;
                        if nb == 1 {
                            break 'merge End::Copy;
                        }
                        if bcount >= min_gallop {
                            break;
                        }
                    }
                }
                // One run is winning so consistently that galloping may be a huge win. So try that, and continue
                // galloping until (if ever) neither run appears to be winning consistently anymore.
                min_gallop += 1;
                loop {
                    min_gallop -= (min_gallop > 1) as usize;
                    self.min_gallop = min_gallop;
                    let k = na
                        - gallop_right(&self.lt, tmp[at(pb)], &self.a[basea..basea + na], na - 1)?;
                    acount = k;
                    if k > 0 {
                        dest -= k as isize;
                        pa -= k as isize;
                        self.a.copy_within(at(pa + 1)..at(pa + 1) + k, at(dest + 1));
                        na -= k;
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    self.a[at(dest)] = tmp[at(pb)];
                    dest -= 1;
                    pb -= 1;
                    nb -= 1;
                    if nb == 1 {
                        break 'merge End::Copy;
                    }
                    let k = nb - gallop_left(&self.lt, self.a[at(pa)], &tmp[..nb], nb - 1)?;
                    bcount = k;
                    if k > 0 {
                        dest -= k as isize;
                        pb -= k as isize;
                        self.a[at(dest + 1)..at(dest + 1) + k]
                            .copy_from_slice(&tmp[at(pb + 1)..at(pb + 1) + k]);
                        nb -= k;
                        if nb == 1 {
                            break 'merge End::Copy;
                        }
                        // nb==0 is impossible now if the comparison function is consistent, but we can't assume
                        // that it is.
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    self.a[at(dest)] = self.a[at(pa)];
                    dest -= 1;
                    pa -= 1;
                    na -= 1;
                    if na == 0 {
                        break 'merge End::Succeed;
                    }
                    if acount < MIN_GALLOP && bcount < MIN_GALLOP {
                        break;
                    }
                }
                min_gallop += 1; // penalize it for leaving galloping mode
                self.min_gallop = min_gallop;
            }
        };
        match end {
            End::Succeed => {
                if nb > 0 {
                    let start = at(dest - (nb as isize - 1));
                    self.a[start..start + nb].copy_from_slice(&tmp[..nb]);
                }
            }
            End::Copy => {
                // The first element of run b belongs at the front of the merge.
                let from = at(pa + 1 - na as isize);
                self.a
                    .copy_within(from..from + na, at(dest + 1 - na as isize));
                dest -= na as isize;
                self.a[at(dest)] = tmp[at(pb)];
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::timsort;

    /// A tiny deterministic generator, so the test needs no dependency.
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed >> 33
    }

    #[test]
    fn sorts_stably() {
        let mut seed = 7;
        for n in [0, 1, 2, 5, 63, 64, 65, 200, 1000, 5000] {
            for modulus in [3, 50, 1 << 30] {
                let v: Vec<(u64, usize)> = (0..n).map(|i| (lcg(&mut seed) % modulus, i)).collect();
                let mut expected = v.clone();
                expected.sort_by_key(|x| x.0);
                assert_eq!(timsort(v, |a, b| Ok(a.0 < b.0)).unwrap(), expected);
            }
        }
        // Long runs that trigger galloping both ways.
        let v: Vec<u64> = (0..300).chain(100..400).rev().chain(0..500).collect();
        let mut expected = v.clone();
        expected.sort();
        assert_eq!(timsort(v, |a, b| Ok(a < b)).unwrap(), expected);
    }

    #[test]
    fn nan_lands_where_cpython_puts_it() {
        let sort = |v: Vec<f64>| timsort(v, |a: &f64, b| Ok(a < b)).unwrap();
        // Expected values computed with CPython 3.13's sorted().
        assert_eq!(
            format!("{:?}", sort(vec![3.0, f64::NAN, 1.0, 2.0])),
            "[3.0, NaN, 1.0, 2.0]"
        );
        assert_eq!(
            format!("{:?}", sort(vec![2.0, 1.0, f64::NAN, 0.5, 3.0])),
            "[1.0, NaN, 0.5, 2.0, 3.0]"
        );
        // Longer lists go through runs, merges and galloping. Checksum of the permutation
        // `sorted(range(300), key=lambda i: v[i])`: sum((k + 1) * p for k, p in enumerate(perm)).
        let checksum = |v: Vec<f64>| -> usize {
            let perm = timsort((0..v.len()).collect(), |&a: &usize, &b| Ok(v[a] < v[b])).unwrap();
            perm.iter().enumerate().map(|(k, p)| (k + 1) * p).sum()
        };
        let v1 = (0..300)
            .map(|i| {
                if i % 7 == 3 {
                    f64::NAN
                } else {
                    ((i * 37) % 101) as f64
                }
            })
            .collect();
        assert_eq!(checksum(v1), 8969397);
        let v2 = (0..300)
            .map(|i| match i {
                _ if i % 13 == 5 => f64::NAN,
                0..150 => i as f64,
                _ => (400 - i) as f64,
            })
            .collect();
        assert_eq!(checksum(v2), 8387965);
    }
}
