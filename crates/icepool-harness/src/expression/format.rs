//! `str` and `repr` of expressions, pools, deals and compound keeps (`__str__` of
//! expression/multiset_expression.py and its subclasses).

use super::{as_expression, as_tuple_expression, KeepIndexArg, MExpr};
use crate::pool::KeepGen;
use crate::{PErr, PValue};
use icepool_core::evaluator::MultisetSource;
use icepool_core::{Int, Value};

/// `str(x)` (node `mx_str`) for a multiset expression, a `MultiDeal` or a die (what an `int` index of a pool
/// evaluates to). Pools, deals, compound keeps, most operators, mixtures and parameters have a `__str__`; the rest
/// show `object.__repr__`, a memory address, which the port does not make up.
pub fn mx_str(x: PValue) -> Result<PValue, PErr> {
    let text = match &x {
        PValue::Die(_) | PValue::DieWithTruth(..) => {
            return crate::format::format(x, PValue::Value(Value::None))
        }
        PValue::MultiDeal(_) => crate::deck::deal_repr_and_str(&x)?.1,
        _ => match as_expression(&x).or_else(|| as_tuple_expression(&x)) {
            Some(e) => expression_str(&e)?,
            // The node is `str(x)`, which never raises for these.
            None => match &x {
                PValue::Value(v) => icepool_core::value::pystr::py_str(v)?,
                PValue::Deck(d) => crate::deck::deck_str(d)?,
                // `AgainExpression` has no `__str__` (`a op b` with `Again` on either side makes one).
                PValue::Again(_) => return Err(address("AgainExpression")),
                _ => return Err(PErr::Unimplemented("str() of this object".into())),
            },
        },
    };
    Ok(PValue::Value(Value::str(&text)))
}

fn address(class: &str) -> PErr {
    PErr::Unimplemented(format!(
        "str() of a {class}: object.__repr__, a memory address"
    ))
}

/// `str(expression)`. A level each (`value::recursion`).
fn expression_str(e: &MExpr) -> Result<String, PErr> {
    let _level = icepool_core::value::recursion::level()?;
    match e {
        MExpr::Gen(KeepGen::Pool(p)) => pool_str(p.dice(), p.keep_tuple()),
        MExpr::Gen(KeepGen::Deal(d)) => crate::deck::deal_text(d.deck(), d.hand_size()),
        MExpr::Gen(KeepGen::Compound(c)) => compound_str(c.inner_sources(), c.keep_tuple()),
        MExpr::MultiDeal(md) => {
            Ok(crate::deck::deal_repr_and_str(&PValue::MultiDeal(md.clone()))?.1)
        }
        MExpr::Weightless(_) => Err(address("WeightlessGenerator")),
        MExpr::Op(node) => {
            let children = node
                .children
                .iter()
                .map(expression_str)
                .collect::<Result<Vec<_>, _>>()?;
            let keep_index = match &node.keep_index {
                Some(KeepIndexArg::Slice(bounds)) => {
                    let [start, stop, step] = &**bounds;
                    Some(format!(
                        "slice({}, {}, {})",
                        pvalue_repr(start)?,
                        pvalue_repr(stop)?,
                        pvalue_repr(step)?
                    ))
                }
                Some(KeepIndexArg::Given(index)) => Some(pvalue_str(index)?),
                None => None,
            };
            node.operator
                .format_str(&children, keep_index.as_deref())
                .ok_or_else(|| address("MultisetOperator without __str__"))
        }
        // `MultisetParameterBase.__str__`: `self._name`.
        MExpr::Param { name, .. } | MExpr::TupleParam { name, .. } => Ok(name.to_string()),
        MExpr::TupleIndex(_) => Err(address("MultisetTupleIndex")),
        MExpr::TupleSlice(_) => Err(address("MultisetTupleSlice")),
        // `'MultisetMixture({\n' + ',\n'.join(f'{k}: {v}' for k, v in self._inner_expressions.items()) + '})'`
        MExpr::Mixture(inners) => {
            let items = inners
                .iter()
                .map(|(inner, weight)| Ok(format!("{}: {weight}", expression_str(inner)?)))
                .collect::<Result<Vec<_>, PErr>>()?;
            Ok(format!("MultisetMixture({{\n{}}})", items.join(",\n")))
        }
    }
}

/// `Pool.__str__`: a header, then `  {repr(die)} : {count},` per die (in `hash_key` order).
fn pool_str(
    dice: &[(icepool_core::Die<Value>, usize)],
    keep_tuple: &[Int],
) -> Result<String, PErr> {
    let raw_size: usize = dice.iter().map(|(_, count)| count).sum();
    let mut text = format!(
        "Pool of {raw_size} dice with keep_tuple={}\n",
        keep_tuple_repr(keep_tuple)?
    );
    for (die, count) in dice {
        let PValue::Value(Value::Str(die)) = crate::die_methods::repr(PValue::Die(die.clone()))?
        else {
            unreachable!("repr() is a str")
        };
        text += &format!("  {die} : {count},\n");
    }
    Ok(text)
}

/// `CompoundKeepGenerator.__str__`, from its inner generators' sources (which keep their dice or deck and keep
/// tuple).
fn compound_str(inners: &[MultisetSource<Value>], keep_tuple: &[Int]) -> Result<String, PErr> {
    let inners = inners
        .iter()
        .map(|inner| match inner {
            MultisetSource::Pool(p) => pool_str(p.dice(), p.keep_tuple()),
            MultisetSource::Deal(d) => crate::deck::deal_text(d.deck(), d.keep_tuple().len()),
            MultisetSource::CompoundKeep(c) => compound_str(c.inner_sources(), c.keep_tuple()),
            _ => Err(address("inner generator")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "CompoundKeep([{}], keep_tuple={})",
        inners.join(", "),
        keep_tuple_repr(keep_tuple)?
    ))
}

fn keep_tuple_repr(keep_tuple: &[Int]) -> Result<String, PErr> {
    let t = Value::tuple(keep_tuple.iter().map(Value::from).collect());
    Ok(icepool_core::value::pystr::py_repr(&t)?)
}

/// `repr()` of a keep index or one of its elements (or of the outcome of a `Break`).
pub(crate) fn pvalue_repr(x: &PValue) -> Result<String, PErr> {
    let items = |xs: &[PValue]| xs.iter().map(pvalue_repr).collect::<Result<Vec<_>, _>>();
    Ok(match x {
        PValue::Value(v) => icepool_core::value::pystr::py_repr(v)?,
        PValue::Ellipsis => "Ellipsis".into(),
        PValue::List(xs) => format!("[{}]", items(xs)?.join(", ")),
        PValue::Tuple(xs) if xs.len() == 1 => format!("({},)", items(xs)?[0]),
        PValue::Tuple(xs) => format!("({})", items(xs)?.join(", ")),
        PValue::Die(_) | PValue::DieWithTruth(..) => {
            let PValue::Value(Value::Str(s)) = crate::die_methods::repr(x.clone())? else {
                unreachable!("repr() is a str")
            };
            s.to_string()
        }
        _ => return Err(PErr::Unimplemented("repr() of this keep index".into())),
    })
}

/// `str()` of a keep index: a value's or a die's own `str`, else its `repr`.
pub(crate) fn pvalue_str(x: &PValue) -> Result<String, PErr> {
    match x {
        PValue::Value(_) | PValue::Die(_) | PValue::DieWithTruth(..) => {
            match crate::format::format(x.clone(), PValue::Value(Value::None))? {
                PValue::Value(Value::Str(s)) => Ok(s.to_string()),
                _ => unreachable!("str() is a str"),
            }
        }
        x => pvalue_repr(x),
    }
}
