//! The operator nodes, named after Python's `operator` module (harness/grammar/basic.py,
//! operators.py): `a <op> b`, `-a`, `n @ die` and `round` with dice, `Again` expressions and plain
//! values on either side, and the operators of multiset expressions and decks.

use super::dice::die_arg;
use super::wrong_type;
use crate::again::{self, AgainOp};
use crate::expression::{as_expression, as_tuple_expression};
use crate::{Args, PErr, PValue};
use icepool_core::value::{BinOp, UnOp};
use icepool_core::{Die, Quantity, Value};

use super::Dispatch;

/// The operators nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "matmul" => args.with(["n", "die"], |[n, die]| matmul(n, die)),
        "round" => args.with(["a", "ndigits"], |[a, ndigits]| round(a, ndigits)),
        _ => {
            if let Some(bin) = binary_op(op) {
                args.with(["a", "b"], |[a, b]| binary(bin, a, b))
            } else if let Some(un) = unary_op(op) {
                args.with(["a"], |[a]| unary(un, a))
            } else {
                return Err(args);
            }
        }
    })
}

/// The binary operator nodes (harness/grammar/basic.py, operators.py), named after Python's `operator` module.
pub(crate) fn binary_op(op: &str) -> Option<BinOp> {
    use BinOp::*;
    Some(match op {
        "add" => Add,
        "sub" => Sub,
        "mul" => Mul,
        "truediv" => TrueDiv,
        "floordiv" => FloorDiv,
        "mod" => Mod,
        "pow" => Pow,
        "lshift" => LShift,
        "rshift" => RShift,
        "and" => And,
        "or" => Or,
        "xor" => Xor,
        "lt" => Lt,
        "le" => Le,
        "gt" => Gt,
        "ge" => Ge,
        _ => return None,
    })
}

pub(crate) fn unary_op(op: &str) -> Option<UnOp> {
    use UnOp::*;
    Some(match op {
        "neg" => Neg,
        "pos" => Pos,
        "invert" => Invert,
        "abs" => Abs,
        "trunc" => Trunc,
        "floor" => Floor,
        "ceil" => Ceil,
        _ => return None,
    })
}

/// `implicit_convert_to_die`: a die stays as it is, anything else becomes `Die([x])` (which raises for `None`, and
/// for a list, which is unhashable; `Reroll` gives the empty die).
pub(crate) fn to_die(x: PValue) -> Result<Die<Value>, PErr> {
    match x {
        PValue::Die(d) | PValue::DieWithTruth(d, _) => Ok(d),
        // `Die([expr])` cannot sort its one outcome: an expression's `<` is an evaluation, a die with no truth
        // value (a multiset tuple expression has no `<`).
        PValue::Pool(_) | PValue::Deal(_) | PValue::MultiDeal(_) | PValue::Expr(_) => {
            wrong_type("Items do not appear to be sortable.")
        }
        x => Ok(Die::new(vec![die_arg(x)?], vec![Quantity::ONE])?),
    }
}

/// A `DieWithTruth` is a `Die` subclass: as an operand of the arithmetic operators it is just its die. (One
/// reaches them when an `Again` expression built with `==` is evaluated inside a bigger one: `(Again == 1) + 2`,
/// `2 @ (Again != 3)`.)
fn plain_die(x: PValue) -> PValue {
    match x {
        PValue::DieWithTruth(d, _) => PValue::Die(d),
        x => x,
    }
}

/// `a <op> b` in Python. With a die on the left, `Die.__op__` converts the right operand. With a die only on the
/// right, the left operand's own `__op__` returns `NotImplemented` (no outcome type knows about dice), so Python
/// calls the die's reflected method: `__radd__` and friends convert the left operand and keep the order
/// (`3 - die` is `Die([3]) - die`), while a comparison reflects (`3 < die` is `die > 3`). A multiset expression or
/// a deck has operators of its own (`multiset_binary`).
pub(crate) fn binary(op: BinOp, a: PValue, b: PValue) -> Result<PValue, PErr> {
    let apply = |x: &Die<Value>, op: BinOp, y: &Die<Value>| {
        x.binary_operator(y, |p, q| p.binary(op, q))
            .map(PValue::Die)
            .map_err(PErr::from)
    };
    let (a, b) = (plain_die(a), plain_die(b));
    // A deck's `+ - & | ^` count the other operand (`Counter(other)`) and a multiset expression's `//` is
    // `count_subset` for anything but an int: neither returns `NotImplemented`, so they come before even an Again
    // expression's reflected operator.
    let counts_other = matches!(op, BinOp::Add | BinOp::Sub | BinOp::And | BinOp::Or | BinOp::Xor);
    if (counts_other && matches!(a, PValue::Deck(_))) || (op == BinOp::FloorDiv && as_expression(&a).is_some()) {
        return multiset_binary(op, a, b);
    }
    match (a, b) {
        // `AgainExpression` defines every operator (and dice return NotImplemented for it), so an expression on
        // either side builds a bigger expression. A comparison with the expression on the right reflects.
        (a @ PValue::Again(_), b) => again::apply(AgainOp::Binary(op), vec![a, b]),
        // Except that `str % x` is printf-style formatting, which runs before the expression's `__rmod__`. The
        // expression is neither a tuple nor a mapping, so it is the one argument: without a conversion specifier
        // it is left over. (With one, `%s` would format its repr, which holds an address.)
        (PValue::Value(Value::Str(s)), PValue::Again(_)) if op == BinOp::Mod => {
            if s.contains('%') {
                Err(PErr::Unimplemented(
                    "str % Again with conversion specifiers".into(),
                ))
            } else {
                wrong_type("not all arguments converted during string formatting")
            }
        }
        (a, b @ PValue::Again(_)) => match op.reflected_comparison() {
            Some(reflected) => again::apply(AgainOp::Binary(reflected), vec![b, a]),
            None => again::apply(AgainOp::Binary(op), vec![a, b]),
        },
        (PValue::Die(a), b) => apply(&a, op, &to_die(b)?),
        // A multiset expression's or deck's own operator, which hands over to the other operand where Python does
        // (`NotImplemented`). A deck's `* // %` take an int only: a die on the right answers them.
        (a, b) if has_operator(op, &a) && !(matches!(a, PValue::Deck(_)) && is_die(&b)) => {
            multiset_binary(op, a, b)
        }
        // The exception: `str % die` is printf-style formatting with the die as the argument, and a die is a
        // Mapping, which formatting accepts even when the format string never uses it.
        (PValue::Value(Value::Str(s)), PValue::Die(_)) if op == BinOp::Mod => Ok(PValue::Value(
            icepool_core::value::ops::str_format_mapping(&s)?,
        )),
        // `Symbols.__floordiv__` takes any iterable as `count_subset`, and a die is one (a mapping: its quantities
        // are the counts).
        (PValue::Value(Value::Symbols(s)), PValue::Die(d) | PValue::DieWithTruth(d, _))
            if op == BinOp::FloorDiv =>
        {
            let items = d
                .items()
                .iter()
                .map(|(o, q)| (o.clone(), Value::Int(q.clone().into())))
                .collect();
            Ok(PValue::Value(s.count_subset_items(items, None)?))
        }
        (a, PValue::Die(b)) => match op.reflected_comparison() {
            Some(reflected) => apply(&b, reflected, &to_die(a)?),
            None => apply(&to_die(a)?, op, &b),
        },
        (PValue::Value(a), PValue::Value(b)) => Ok(PValue::Value(a.binary(op, &b)?)),
        // The reflected operators of a multiset expression or deck on the right (or Python's TypeError).
        (a, b) if is_multiset(&a) || is_multiset(&b) => multiset_binary(op, a, b),
        // `Reroll` is an enum member: it has no operators (`==` is the `eq` node's).
        (
            PValue::Reroll | PValue::Restart | PValue::Value(_),
            PValue::Reroll | PValue::Restart | PValue::Value(_),
        ) => wrong_type(format!(
            "unsupported operand type(s) for {}: 'RerollType'",
            op.symbol()
        )),
        _ => Err(PErr::Unimplemented(
            "operators on lists, tuples of dice, functions".into(),
        )),
    }
}

fn is_die(x: &PValue) -> bool {
    matches!(x, PValue::Die(_) | PValue::DieWithTruth(..))
}

/// A multiset expression, a multiset tuple expression or a deck.
fn is_multiset(x: &PValue) -> bool {
    as_expression(x).is_some() || as_tuple_expression(x).is_some() || matches!(x, PValue::Deck(_))
}

/// Whether `x.__op__` exists: a multiset expression has `+ - & | ^ * // %` and the subset comparisons
/// (multiset_expression.py), a deck `+ - & | ^ * // %` (deck.py); a multiset tuple expression has none.
fn has_operator(op: BinOp, x: &PValue) -> bool {
    use BinOp::*;
    let arithmetic = matches!(op, Add | Sub | And | Or | Xor | Mul | FloorDiv | Mod);
    (as_expression(x).is_some() && (arithmetic || matches!(op, Lt | Le | Gt | Ge)))
        || (matches!(x, PValue::Deck(_)) && arithmetic)
}

/// `a <op> b` with a multiset expression, a multiset tuple expression or a deck as an operand, once no die or Again
/// expression answers first: the left operand's own operator, else the right one's reflected operator (the
/// expression and deck nodes do that dispatch), else Python's TypeError.
fn multiset_binary(op: BinOp, a: PValue, b: PValue) -> Result<PValue, PErr> {
    use BinOp::*;
    let symbol = op.symbol();
    let expression = |x: &PValue| as_expression(x).is_some();
    let deck = |x: &PValue| matches!(x, PValue::Deck(_));
    let name = |s: &str| PValue::Value(Value::str(s));
    let none = || PValue::Value(Value::None);
    match op {
        Add | Sub | And | Or | Xor if deck(&a) => crate::deck::deck_op(name(symbol), a, b),
        Add | Sub | And | Or | Xor if expression(&a) || expression(&b) => {
            crate::expression::binary_op(symbol, a, b)
        }
        // `__radd__`, `__rsub__`, `__rand__`, `__ror__` (and no `__rxor__`).
        Add | Sub | And | Or | Xor if deck(&b) => crate::deck::deck_op(name(symbol), a, b),
        Mul | FloorDiv | Mod if expression(&a) => {
            let method = match op {
                Mul => "mul",
                FloorDiv => "floordiv",
                _ => "mod",
            };
            crate::expression::counts(a, method, none(), b)
        }
        Mul | FloorDiv | Mod if deck(&a) => crate::deck::deck_mul(a, name(symbol), b),
        // `__rmul__` (an int only); an expression's `__rfloordiv__` is `count_subset` the other way round.
        Mul if expression(&b) => crate::expression::counts(b, "rmul", none(), a),
        Mul if deck(&b) => crate::deck::deck_mul(b, name("r*"), a),
        FloorDiv if expression(&b) => crate::evaluate::rfloordiv(a, b),
        // `str % x` is printf-style formatting, and these are mappings to it (they have `__getitem__`).
        Mod => match &a {
            PValue::Value(Value::Str(s)) => Ok(PValue::Value(
                icepool_core::value::ops::str_format_mapping(s)?,
            )),
            _ => wrong_type(format!("unsupported operand type(s) for {symbol}")),
        },
        Lt | Le | Gt | Ge if expression(&a) || expression(&b) => {
            crate::evaluate::compare_operator(symbol, a, b)
        }
        Lt | Le | Gt | Ge => wrong_type(format!("'{symbol}' not supported between these instances")),
        _ => wrong_type(format!("unsupported operand type(s) for {symbol}")),
    }
}

/// `-a`, `+a`, `~a`, `abs(a)`, `math.trunc(a)`, `math.floor(a)`, `math.ceil(a)`: `Die.unary_operator` on a die.
/// A multiset expression has `-` and `+` (`-1 * self`, `keep_counts('>=', 0)`).
pub(crate) fn unary(op: UnOp, a: PValue) -> Result<PValue, PErr> {
    match plain_die(a) {
        // `AgainExpression` has `-`, `+`, `~` and `abs`, but no `__trunc__`, `__floor__`, `__ceil__`.
        a @ PValue::Again(_) if matches!(op, UnOp::Neg | UnOp::Pos | UnOp::Invert | UnOp::Abs) => {
            again::apply(AgainOp::Unary(op), vec![a])
        }
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.unary(op))?)),
        PValue::Value(v) => Ok(PValue::Value(v.unary(op)?)),
        a if as_expression(&a).is_some() && matches!(op, UnOp::Neg | UnOp::Pos) => {
            let method = if op == UnOp::Neg { "neg" } else { "pos" };
            let none = PValue::Value(Value::None);
            crate::expression::counts(a, method, none.clone(), none)
        }
        _ => wrong_type("bad operand type for a unary operator"),
    }
}

/// `n @ die`: roll `n`, then roll `die` that many times and sum the outcomes. An int `n` has no `@`, so Python
/// calls `Die.__rmatmul__`, which converts it with `implicit_convert_to_die`: `Die([n]) @ die`. With an `Again`
/// expression on either side (dice return NotImplemented for it) the result is a bigger expression.
pub(crate) fn matmul(n: PValue, die: PValue) -> Result<PValue, PErr> {
    let (n, die) = match (plain_die(n), plain_die(die)) {
        (n @ PValue::Again(_), die) | (n, die @ PValue::Again(_)) => {
            return again::apply(AgainOp::MatMul, vec![n, die])
        }
        (PValue::Die(n), die) => (n, to_die(die)?),
        (n, PValue::Die(die)) => (to_die(n)?, die),
        _ => return wrong_type("unsupported operand type(s) for @"),
    };
    Ok(PValue::Die(icepool_core::value::matmul_dice(&n, &die)?))
}

/// `round(a, ndigits)`. For a die, `Die.round` rounds each outcome, so a bad `ndigits` only raises if there is an
/// outcome to round.
fn round(a: PValue, ndigits: PValue) -> Result<PValue, PErr> {
    // A die or a list as ndigits is not an int, so rounding any outcome with it raises; but `Die.round` on an
    // empty die rounds nothing and returns it. A str stands in: `Value::round` raises for any non-int ndigits.
    let nd = match &ndigits {
        PValue::Value(v) => v.clone(),
        _ => Value::str(""),
    };
    match a {
        PValue::Die(d) => Ok(PValue::Die(d.unary_operator(|x| x.round(&nd))?)),
        PValue::Value(v) => Ok(PValue::Value(v.round(&nd)?)),
        _ => wrong_type("type doesn't define __round__ method"),
    }
}
