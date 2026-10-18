//! Decks, deals and multi-deals (harness/grammar/deck.py).

use crate::deck;
use crate::multi_deal;
use crate::Args;

use super::Dispatch;

/// The deck nodes: the node's result, or the arguments back if `op` is not one of them.
pub(super) fn call<'a>(op: &str, args: Args<'a>) -> Dispatch<'a> {
    Ok(match op {
        "deck" => args.with(["outcomes", "times"], |[outcomes, times]| {
            deck::deck(outcomes, times)
        }),
        "deck_op" => args.with(["operator", "a", "b"], |[op, a, b]| deck::deck_op(op, a, b)),
        "deck_mul" => args.with(["deck", "operator", "n"], |[d, op, n]| {
            deck::deck_mul(d, op, n)
        }),
        "deck_map" => args.with(["deck", "repl", "star"], |[d, repl, star]| {
            deck::deck_map(d, repl, star)
        }),
        "deck_sequence" => args.with(["deck", "deals"], |[d, deals]| {
            deck::deck_sequence(d, deals)
        }),
        "deck_method" => args.with(["deck", "name", "arg"], |[d, name, arg]| {
            deck::deck_method(d, name, arg)
        }),
        "deal_str" => args.with(["deal"], |[d]| deck::deal_str(d)),
        "deck_info" => args.with(["deck"], |[d]| deck::deck_info(d)),
        "deck_pop" => args.with(["deck", "max"], |[d, max]| deck::deck_pop(d, max)),
        "deck_stat" => args.with(["deck", "stat"], |[d, stat]| deck::deck_stat(d, stat)),
        "deck_pad_to_denominator" => args.with(
            ["deck", "denominator", "outcome"],
            |[d, denominator, outcome]| deck::deck_pad_to_denominator(d, denominator, outcome),
        ),
        "deal" => args.with(["deck", "hand_size"], |[d, hand_size]| {
            deck::deal(d, hand_size)
        }),
        "multi_deal" => args.with(["deck", "hand_sizes"], |[d, hand_sizes]| {
            deck::deal(d, hand_sizes)
        }),
        "deal_groups" => args.with(["deck", "hand_groups"], |[d, groups]| {
            multi_deal::deal_groups(d, groups)
        }),
        "multi_deal_info" => args.with(["multi_deal"], |[d]| multi_deal::multi_deal_info(d)),
        "multi_deal_pop" => args.with(["multi_deal", "order", "outcome"], |[d, order, outcome]| {
            multi_deal::multi_deal_pop(d, order, outcome)
        }),
        "deal_info" => args.with(["deal"], |[d]| deck::deal_info(d)),
        "deal_pop" => args.with(["deal", "order", "outcome"], |[d, order, outcome]| {
            deck::deal_pop(d, order, outcome)
        }),
        _ => return Err(args),
    })
}
