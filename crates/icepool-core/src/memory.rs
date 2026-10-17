//! Size checks for allocations whose size comes from user input (`d(sides)`, `die_sequence(die, rolls)`, a list
//! repeated `n` times, a cartesian product's size, ...).
//!
//! Rust aborts the process when an allocation fails (`handle_alloc_error`), and panics on a capacity that
//! overflows `isize`; in WASM both are traps. Python raises `MemoryError` instead, so every allocation sized by user
//! input goes through here: refused past [`MAX_ALLOC_BYTES`], else reserved with `try_reserve_exact`, a failure of
//! which is `MemoryError` too. Allocations sized by data that already exists (a die's own outcomes, say) cannot be
//! bigger than what was allocated before and need no check.
//!
//! Below the cap Python and the port both answer given memory and time, so parity holds on native and Python
//! builds: the cap only refuses sizes no real machine holds. On wasm32 the cap is much lower, because the whole
//! instance lives in a 4 GiB address space and the result's caches and copies need room too.

use dashu_base::BitTest;

use crate::error::{Error, Result};

/// The largest allocation the port makes for a size given by user input. wasm32: 256 MiB, so that the result and
/// what is built from it (sorted copies, an outcome index, cumulative quantities) still fit in 4 GiB. Native: 64 GiB.
/// (Two `cfg`s rather than one `if`: `1 << 36` does not even compile as a 32-bit `usize`.)
#[cfg(target_pointer_width = "32")]
pub const MAX_ALLOC_BYTES: usize = 1 << 28;
#[cfg(not(target_pointer_width = "32"))]
pub const MAX_ALLOC_BYTES: usize = 1 << 36;

/// Python's `MemoryError`, for `n` elements of what is being built.
pub fn memory_error(what: &str, n: impl std::fmt::Display) -> Error {
    Error::Memory(format!("{what} of {n} elements"))
}

/// `n` (any integer type, or a `checked_mul` that overflowed as `None`) as a length of `T`s the port may allocate:
/// `MemoryError` if `n * size_of::<T>()` passes [`MAX_ALLOC_BYTES`].
pub fn check_len<T, N>(n: Option<N>, what: &str) -> Result<usize>
where
    N: TryInto<usize> + Copy + std::fmt::Display,
{
    let fits = |n: usize| n.checked_mul(size_of::<T>().max(1)).is_some_and(|b| b <= MAX_ALLOC_BYTES);
    match n {
        Some(k) => match k.try_into() {
            Ok(len) if fits(len) => Ok(len),
            _ => Err(memory_error(what, k)),
        },
        None => Err(Error::Memory(format!("{what}: too many elements"))),
    }
}

/// An empty `Vec` with room for `n` elements, or `MemoryError` (past the cap, or if the allocator refuses).
pub fn try_with_capacity<T, N>(n: Option<N>, what: &str) -> Result<Vec<T>>
where
    N: TryInto<usize> + Copy + std::fmt::Display,
{
    let len = check_len::<T, N>(n, what)?;
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| memory_error(what, len))?;
    Ok(v)
}

/// `[x] * n` as a `Vec`, or `MemoryError`.
pub fn try_repeat<T: Clone, N>(x: T, n: Option<N>, what: &str) -> Result<Vec<T>>
where
    N: TryInto<usize> + Copy + std::fmt::Display,
{
    let mut v = try_with_capacity(n, what)?;
    let len = v.capacity();
    v.extend(std::iter::repeat_n(x, len));
    Ok(v)
}

/// `iter` collected into a `Vec` of exactly `n` elements reserved up front, or `MemoryError`.
pub fn try_collect<T, N>(n: Option<N>, iter: impl IntoIterator<Item = T>, what: &str) -> Result<Vec<T>>
where
    N: TryInto<usize> + Copy + std::fmt::Display,
{
    let mut v = try_with_capacity(n, what)?;
    v.extend(iter);
    Ok(v)
}

/// `base ** exp` of a quantity, whose result has about `exp * bits(base)` bits: `MemoryError` past the cap, where
/// computing it would abort.
pub fn check_pow(base: &crate::Quantity, exp: usize, what: &str) -> Result<()> {
    if *base <= crate::Quantity::ONE {
        return Ok(());
    }
    let bytes = (base.bit_len() as u64).checked_mul(exp as u64).map(|bits| bits / 8);
    check_len::<u8, u64>(bytes, what)
        .map(drop)
        .map_err(|_| Error::Memory(format!("{what}: a result of {} bytes", bytes.map_or("too many".into(), |b| b.to_string()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        // Past the cap, a size that overflows usize or isize, a negative size: MemoryError, never an abort.
        let huge = MAX_ALLOC_BYTES / 8 + 1;
        for n in [Some(huge as u64), Some(u64::MAX), None] {
            assert!(matches!(try_with_capacity::<u64, u64>(n, "x"), Err(Error::Memory(_))), "{n:?}");
        }
        assert!(matches!(try_repeat(0u8, Some(-1i64), "x"), Err(Error::Memory(_))));
        assert!(matches!(check_len::<(), i64>(Some(i64::MAX), "x"), Err(Error::Memory(_))));
        assert_eq!(try_repeat(7u8, Some(3usize), "x").unwrap(), vec![7, 7, 7]);
        assert_eq!(try_collect(Some(2u32), [1, 2], "x").unwrap(), vec![1, 2]);
        assert_eq!(check_len::<u64, usize>(Some(MAX_ALLOC_BYTES / 8), "x").unwrap(), MAX_ALLOC_BYTES / 8);
        let q = |x: u32| crate::Quantity::from(x);
        assert!(check_pow(&q(1), usize::MAX, "x").is_ok());
        assert!(check_pow(&q(255), MAX_ALLOC_BYTES, "x").is_ok());
        assert!(matches!(check_pow(&q(256), MAX_ALLOC_BYTES, "x"), Err(Error::Memory(_))));
        assert!(matches!(check_pow(&q(2), usize::MAX, "x"), Err(Error::Memory(_))));
    }
}
