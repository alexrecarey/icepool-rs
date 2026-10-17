// Exceptions: what icepool raises, as JS `Error` subclasses.

/** The core's error variants (`icepool_core::Error`), plus `Panic` for a bug in the core (a trap, after which the
 * instance is still usable). Each maps to the exception Python icepool raises ({@link IcepoolError.pythonType}):
 *
 * | kind                                      | pythonType                                          |
 * |:------------------------------------------|:----------------------------------------------------|
 * | `InvalidArgument`                         | `ValueError`                                        |
 * | `WrongType`, `Incomparable`, `Unhashable` | `TypeError`                                         |
 * | `MissingAttribute`                        | `AttributeError`                                    |
 * | `IndexOutOfRange`                         | `IndexError`                                        |
 * | `MissingKey`                              | `KeyError`                                          |
 * | `DivisionByZero`                          | `ZeroDivisionError`                                 |
 * | `ConflictingOrder`                        | `ConflictingOrderError`                             |
 * | `UninferrableSize`                        | `OrderError`                                        |
 * | `UnsupportedOrder`                        | `UnsupportedOrder`                                  |
 * | `RecursionLimit`                          | `RecursionError`                                    |
 * | `Overflow`                                | `OverflowError`                                     |
 * | `TooLarge` (a size no memory holds)       | `MemoryError`                                       |
 * | `NotImplementedUpstream`                  | `NotImplementedError` (icepool does not do this)    |
 * | `Unported`                                | `NotImplementedError` (Python would, not ported)    |
 * | `Callback`                                | `Exception` (a callback's own error is rethrown)    |
 * | `Panic`                                   | `PanicException`                                    |
 *
 * @example
 * ```ts
 * import { d, Die, IcepoolError, type ErrorKind } from "icepool-wasm";
 * const kindOf = (f: () => unknown): ErrorKind | undefined => {
 *   try { f(); } catch (e) { return (e as IcepoolError).kind; }
 * };
 * kindOf(() => d(0)); //=> "InvalidArgument"
 * kindOf(() => new Die([]).mean()); //=> "DivisionByZero"
 * ```
 */
export type ErrorKind =
  | "InvalidArgument"
  | "WrongType"
  | "MissingAttribute"
  | "IndexOutOfRange"
  | "MissingKey"
  | "Incomparable"
  | "Unhashable"
  | "DivisionByZero"
  | "ConflictingOrder"
  | "UninferrableSize"
  | "UnsupportedOrder"
  | "RecursionLimit"
  | "Overflow"
  | "TooLarge"
  | "NotImplementedUpstream"
  | "Unported"
  | "Callback"
  | "Panic";

/** An exception icepool raised. `kind` is what went wrong in the core's terms; `pythonType` is the exception
 * Python icepool raises for it (`ValueError`, `TypeError`, `MemoryError`, ..., `PanicException` for a panic), which
 * is also the error's `name`. An exception thrown by your own callback reaches you unchanged, not as this.
 *
 * @example
 * ```ts
 * d(0); //=> throws ValueError
 * const error = (f: () => unknown) => { try { f(); } catch (e) { return e; } };
 * const e = error(() => new Die([]).mean()) as IcepoolError;
 * e instanceof IcepoolError; //=> true
 * [e.name, e.pythonType, e.kind]; //=> ["ZeroDivisionError", "ZeroDivisionError", "DivisionByZero"]
 * ```
 */
export class IcepoolError extends Error {
  /** What went wrong, in the core's terms. */
  readonly kind: ErrorKind;
  /** The Python exception's name (`"ValueError"`, ...), also the error's `name`. */
  readonly pythonType: string;

  /** An error of this kind; `options.cause` as for `Error`. The API makes these; code using it rarely needs to. */
  constructor(kind: ErrorKind, pythonType: string, message = "", options?: { cause?: unknown }) {
    super(message, options);
    this.name = pythonType;
    this.kind = kind;
    this.pythonType = pythonType;
  }

  /** Python would answer, but the port does not cover the case yet. */
  get unimplemented(): boolean {
    return this.kind === "Unported";
  }
}

/** icepool's `OrderError`: an unrecoverable error related to outcome orderings; an evaluation cannot run in the
 * order it needs. On its own (`kind` `UninferrableSize`), it is thrown where an operator needs the size of its right
 * operand to choose an order and the size cannot be inferred.
 *
 * @example
 * ```ts
 * const e = new ConflictingOrderError("ConflictingOrder", "ConflictingOrderError", "conflicting orders");
 * e instanceof OrderError; //=> true
 * e.name; //=> "ConflictingOrderError"
 * ```
 */
export class OrderError extends IcepoolError {}

/** icepool's `ConflictingOrderError`: two conflicting mandatory outcome orderings were encountered; the inputs of an
 * evaluation ask for opposite orders.
 *
 * @example
 * ```ts
 * const sum = () => d(6).pool(1).forceOrder(Order.Ascending).forceOrder(Order.Descending).sum();
 * sum(); //=> throws ConflictingOrderError
 * ```
 */
export class ConflictingOrderError extends OrderError {}

/** icepool's `UnsupportedOrder`: the given order is not supported under the current context, but retrying with the
 * opposite order may still succeed. Throw it from an evaluator's method when it cannot handle the order it is given:
 * the evaluation retries in the other order (if it has not been tried already). Unlike Python's, which derives from
 * `OrderException` only, it is an `OrderError` here.
 *
 * @example
 * ```ts
 * // The highest die of a pool, which needs to see the outcomes in descending order.
 * const highestDie = evaluator({
 *   nextState(state, order, outcome, count) {
 *     if (order !== Order.Descending) throw new UnsupportedOrder();
 *     if (state !== null) return state as number; // the highest is already found
 *     return (count as number) > 0 ? outcome : null;
 *   },
 * });
 * highestDie.evaluate(d(6).pool(3)).mean().toString(); //=> "119/24"
 * ```
 */
export class UnsupportedOrder extends OrderError {
  /** `UnsupportedOrder(message)`: `kind` and `pythonType` are both `"UnsupportedOrder"`. */
  constructor(message = "") {
    super("UnsupportedOrder", "UnsupportedOrder", message);
  }
}

/** The exception for a core error (`kind`, `pythonType` and message as src/api/error.rs gives them). */
export function makeError(kind: ErrorKind, pythonType: string, message: string): IcepoolError {
  switch (kind) {
    case "ConflictingOrder":
      return new ConflictingOrderError(kind, pythonType, message);
    case "UninferrableSize":
      return new OrderError(kind, pythonType, message);
    case "UnsupportedOrder":
      return new UnsupportedOrder(message);
    default:
      return new IcepoolError(kind, pythonType, message);
  }
}
