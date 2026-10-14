"""Program-language node definitions. The loop adds modules here as features land."""
import signal


class _PeekTimeout(Exception):
    pass


def _alarm(signum, frame):
    raise _PeekTimeout()


def peek(program, seconds=2):
    """evaluate(program) for a generator that asks the oracle about a subprogram (pool.py's _die_without_nan, ...).

    Generators run outside the fuzzer's oracle timeout, and some programs never finish (map(repeat='inf') over a
    process with infinitely many states explores forever, in Python too): give up after `seconds` and raise
    TimeoutError, which those generators treat like any other raise (the program is kept; the fuzzer's oracle then
    times out on it and skips it)."""
    from harness.core.grammar_api import evaluate
    previous = signal.signal(signal.SIGALRM, _alarm)
    signal.alarm(seconds)
    try:
        return evaluate(program)
    except _PeekTimeout:
        raise TimeoutError(f"peek: gave up after {seconds}s") from None
    finally:
        signal.alarm(0)
        signal.signal(signal.SIGALRM, previous)
