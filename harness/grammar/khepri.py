"""khepri-f2f's whole dice engine as one node: the sweep (check 5).

Python side: harness/core/khepri/f2f.py, a verbatim copy of khepri-f2f's src/engine/f2f.py (protected). The result
is the exact Die of (winner, wounds) that f2f.calculate turns into percentages, so the comparison is exact rather
than float-rounded.
"""
from harness.core.grammar_api import node
from harness.core.khepri import f2f


def side(g):
    r = g.rng
    saves = r.choice([0, 1, 1, 1, 2, 3])
    return {
        'successValue': r.choice([0, 1, 5, 8, 10, 11, 12, 13, 14, 15, 17, 19, 20, 21, 23, 26, 30]),
        'burst': r.choice([0, 1, 1, 2, 2, 3, 3, 4, 5, 6]),
        'bonusBurst': r.choice([0, 0, 0, 1, 2, 3]),
        'template': False,
        'saveValue': r.randint(0, 20),
        'saves': saves,
        'critSave': saves > 0 and r.random() < 0.85,
        'woundsPerFailure': r.choice([1, 1, 1, 2]),
        'cont': r.random() < 0.15,
        'secondarySave': r.randint(0, 20) if r.random() < 0.1 else None,
    }


def _gen(g, depth):
    a, b = side(g), side(g)
    r = g.rng.random()
    if r < 0.07:
        a['template'] = True
    elif r < 0.14:
        b['template'] = True
    p = {**{k + 'A': v for k, v in a.items()}, **{k + 'B': v for k, v in b.items()},
         'fixedFaceToFace': g.rng.random() < 0.07}
    # Keep the oracle quick: the heaviest corner (big bursts, Continuous, many saves on both sides) can take ~0.5 s.
    heavy = sum(p[f'burst{s}'] + p[f'bonusBurst{s}'] for s in 'AB')
    if heavy > 12:
        for s in 'AB':
            p[f'cont{s}'] = False
    return {"op": "khepri", "input": p}


@node("khepri", "die", gen=_gen, leaf=True, lazy=("input",), weight=0.0001)
def khepri(input):
    """khepri's engine: Die of (winner, wounds) with winner 'active' / 'reactive' / 'fail'."""
    return f2f.outcomes(input, f2f.roll(input))
