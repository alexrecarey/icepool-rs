//! The keep family's single-die shortcuts (population/die.py `_lowest_single` / `_highest_single`, population/keep.py
//! of the same names) and the pool compositions of `Die.explode_to_pool` / `Die.reroll_to_pool`, under
//! js/src/die.ts and js/src/functions.ts. The TypeScript does Python's argument juggling (the slices, `split`, the
//! early `self.pool(rolls)` returns, `map_to_pool` over the composition), as the Python facade does.

use icepool_core::value::reroll_composition as core_reroll_composition;
use icepool_core::value::{explode_counts as core_explode_counts, matmul_dice, DieArg};
use icepool_core::{Die, Error, Quantity, Value};
use wasm_bindgen::prelude::*;

use super::convert::{int_from_js, value_from_js};
use super::die::WasmDie;
use super::error::{JsResult, OrThrow};

/// `n @ die` with a Python int (or any value) `n`: `Die([n]) @ die` (`Die.__rmatmul__`).
fn rmatmul(n: &Value, die: &Die<Value>) -> Result<Die<Value>, Error> {
    let n = Die::new(vec![DieArg::Outcome(n.clone())], vec![Quantity::ONE])?;
    matmul_dice(&n, die)
}

#[wasm_bindgen]
impl WasmDie {
    /// `die._highest_single(rolls)` (`highest`) or `_lowest_single(rolls)` for an int `rolls > 0` (the TypeScript
    /// makes `rolls == 0` `zero().simplify()`): `from_cumulative` over each cumulative quantity `** rolls`. The
    /// core refuses results no memory holds (`MemoryError`), and a die whose quantities are all 0 or 1 answers for
    /// any number of rolls, as Python does.
    #[wasm_bindgen(js_name = keepSingle)]
    pub fn keep_single(&self, rolls: JsValue, highest: bool) -> JsResult<WasmDie> {
        let rolls = int_from_js(&rolls, "rolls").or_throw()?;
        let rolls = usize::try_from(&rolls).unwrap_or(usize::MAX);
        let die = if highest {
            self.die.highest_single(rolls)
        } else {
            self.die.lowest_single(rolls)
        };
        Ok(WasmDie::new(die.or_throw()?))
    }
}

/// keep.py `_highest_single(*dice)` (`highest`) or `_lowest_single(*dice)`: roll all the dice (at least one, none
/// empty) and take the highest (lowest) single one. wasm-bindgen takes the dice over, so the TypeScript passes
/// copies.
#[wasm_bindgen(js_name = keepSingleOf)]
pub fn keep_single_of(dice: Vec<WasmDie>, highest: bool) -> JsResult<WasmDie> {
    let dice: Vec<Die<Value>> = dice.into_iter().map(|d| d.die.clone()).collect();
    let die = if highest {
        icepool_core::value::highest_single(&dice)
    } else {
        icepool_core::value::lowest_single(&dice)
    };
    Ok(WasmDie::new(die.or_throw()?))
}

/// `explode_to_pool`'s `count_die = rolls @ Die(single_data)` once the TypeScript has split `die` into `explode` and
/// `not_explode` and checked `depth` (an int): a die of `Vector((exploded, not_exploded))` counts.
#[wasm_bindgen(js_name = explodeCounts)]
pub fn explode_counts(
    die: &WasmDie,
    explode: &WasmDie,
    not_explode: &WasmDie,
    rolls: JsValue,
    depth: JsValue,
) -> JsResult<WasmDie> {
    let rolls = value_from_js(&rolls).or_throw()?;
    let depth = int_from_js(&depth, "depth").or_throw()?;
    // Python would run out of time or memory with a depth this large.
    let depth = i64::try_from(&depth)
        .map_err(|_| Error::Unported("a huge explode_to_pool depth".into()))
        .or_throw()?;
    let counts = core_explode_counts(
        &die.die.denominator(),
        &explode.die.denominator(),
        &not_explode.die.denominator(),
        &rolls,
        depth,
        rmatmul,
    );
    Ok(WasmDie::new(counts.or_throw()?))
}

/// `reroll_to_pool`'s `pool_composition` for a finite `max_rerolls` (`depth` already resolved: `'inf'` is
/// `max_rerolls`): a die of `(rerollable, rerolled, rerolled_to_rerollable)` tuples. `rerollable` is the rerollable
/// half of `die` after `split`. The counts are Python values, so all arithmetic on them is `Value`'s.
#[wasm_bindgen(js_name = rerollComposition)]
pub fn reroll_composition(
    die: &WasmDie,
    rerollable: &WasmDie,
    rolls: JsValue,
    max_rerolls: JsValue,
    depth: JsValue,
) -> JsResult<WasmDie> {
    let [rolls, max_rerolls, depth] = [rolls, max_rerolls, depth].map(|x| value_from_js(&x));
    let composition = core_reroll_composition(
        &die.die.denominator(),
        &rerollable.die.denominator(),
        &rolls.or_throw()?,
        &max_rerolls.or_throw()?,
        &depth.or_throw()?,
        rmatmul,
    );
    Ok(WasmDie::new(composition.or_throw()?))
}
