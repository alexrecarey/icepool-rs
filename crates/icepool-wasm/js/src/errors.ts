// Exceptions: what icepool raises, as JS `Error` subclasses.

/** The core's error variants (`icepool_core::Error`), plus `Panic` for a bug in the core (a trap, after which the
 * instance is still usable). */
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
 * is also the error's `name`. */
export class IcepoolError extends Error {
  readonly kind: ErrorKind;
  readonly pythonType: string;

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

/** icepool's `OrderError`: an evaluation cannot run in the order it needs. */
export class OrderError extends IcepoolError {}

/** icepool's `ConflictingOrderError`: the inputs of an evaluation ask for opposite orders. */
export class ConflictingOrderError extends OrderError {}

/** icepool's `UnsupportedOrder`. Throw it from an evaluator's method when it cannot handle the order it is given:
 * the evaluation retries in the other order. */
export class UnsupportedOrder extends OrderError {
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
