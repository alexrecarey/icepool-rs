//! `sin`, `cos`, `atan2` and `exp` exactly as glibc computes them, for targets whose libm differs (WASM). See
//! `pow.rs` for why; CPython's complex power (`_Py_c_pow`) ends in these.
//!
//! Natively the port calls glibc through `f64::sin` etc. On wasm32 these ports stand in: `sin`/`cos`/`atan2` are
//! IBM's Accurate Mathematical Library as glibc 2.43 has it (sysdeps/ieee754/dbl-64/s_sin.c, e_atan2.c,
//! branred.c, dla.h; Copyright (C) 2001-2026 Free Software Foundation, Inc., LGPL-2.1-or-later, see QUESTIONS.md),
//! its x86_64 FMA builds `__sin_fma`, `__cos_fma`, `__atan2_fma` (what glibc picks on an FMA machine, the
//! oracle's). Those are compiled with `-mfma`, and GCC fuses every product whose uses are all additions or
//! subtractions into each of them (a product feeding two subtractions is fused into both) when they are in the same
//! basic block; where both operands of a sum are products, the one computed first is fused. Each fused operation
//! is a `mul_add` here, as read off glibc 2.43's disassembly. `__branred` is shared by all builds and has no FMA.
//! `exp` is Arm's (pow.rs's `exp_inline`, whose contractions are `__exp_fma`'s too). `tests::matches_platform_*`
//! check every function against glibc natively.

use super::pow;
use super::trig_tables::{CIJ, SINCOSTAB};

/// `sin(x)` with the platform's rounding.
pub fn sin(x: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        ibm_sin(x)
    } else {
        x.sin()
    }
}

/// `cos(x)` with the platform's rounding.
pub fn cos(x: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        ibm_cos(x)
    } else {
        x.cos()
    }
}

/// `atan2(y, x)` with the platform's rounding.
pub fn atan2(y: f64, x: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        ibm_atan2(y, x)
    } else {
        y.atan2(x)
    }
}

/// `exp(x)` with the platform's rounding.
pub fn exp(x: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        pow::arm_exp(x)
    } else {
        x.exp()
    }
}

/// `u.i[HIGH_HALF]` and `u.i[LOW_HALF]` of a double, as C's `int`.
fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as u32 as i32
}

fn lo(x: f64) -> i32 {
    x.to_bits() as u32 as i32
}

const fn f(bits: u64) -> f64 {
    f64::from_bits(bits)
}

// usncs.h
const S1: f64 = f(0xbfc5555555555555); // -0x1.5555555555555p-3
const S2: f64 = f(0x3f81111111110ece); // 0x1.1111111110ECEp-7
const S3: f64 = f(0xbf2a01a019db08b8); // -0x1.A01A019DB08B8p-13
const S4: f64 = f(0x3ec71de27b9a7ed9); // 0x1.71DE27B9A7ED9p-19
const S5: f64 = f(0xbe5addffc2fcdf59); // -0x1.ADDFFC2FCDF59p-26
const BIG: f64 = f(0x42c8000000000000); // 0x1.8p45
const HP0: f64 = f(0x3ff921fb54442d18); // pi/2, high part
const HP1: f64 = f(0x3c91a62633145c07); // pi/2, low part
const MP1: f64 = f(0x3ff921fb58000000);
const MP2: f64 = f(0xbe4dde973c000000);
const PP3: f64 = f(0xbc8cb3b398000000);
const PP4: f64 = f(0xbacd747f23e32ed7);
const HPINV: f64 = f(0x3fe45f306dc9c883); // 2/pi
const TOINT: f64 = f(0x4338000000000000); // 0x1.8p52

// s_sin.c, its literals as written (more digits than a double holds).
#[allow(clippy::excessive_precision)]
const SN3: f64 = -1.66666666666664880952546298448555E-01;
#[allow(clippy::excessive_precision)]
const SN5: f64 = 8.33333214285722277379541354343671E-03;
#[allow(clippy::excessive_precision)]
const CS2: f64 = 4.99999999999999999999950396842453E-01;
#[allow(clippy::excessive_precision)]
const CS4: f64 = -4.16666666666664434524222570944589E-02;
#[allow(clippy::excessive_precision)]
const CS6: f64 = 1.38888874007937613028114285595617E-03;

/// `SINCOS_TABLE_LOOKUP`: sin, its correction, cos, its correction at the table point `u` selects.
fn table(u: f64) -> (f64, f64, f64, f64) {
    let k = (lo(u) << 2) as usize;
    let t = |i: usize| f64::from_bits(SINCOSTAB[k + i]);
    (t(0), t(1), t(2), t(3))
}

/// `TAYLOR_SIN`: x - x^3/3! + x^5/5! - x^7/7! + x^9/9! - dx*x^2/2 + dx, the constants s1, s2, ... being the
/// pre-computed values of 1/3!, 1/5! and so on.
fn taylor_sin(xx: f64, x: f64, dx: f64) -> f64 {
    let poly = S5
        .mul_add(xx, S4)
        .mul_add(xx, S3)
        .mul_add(xx, S2)
        .mul_add(xx, S1);
    let t = poly.mul_add(x, -(0.5 * dx)).mul_add(xx, dx);
    x + t
}

/// Given a number partitioned into X and DX, computes the cosine of the number by combining the sin and cos of X
/// (as computed by a variation of the Taylor series) with the values looked up from the sin/cos table.
fn do_cos(x: f64, mut dx: f64) -> f64 {
    if x < 0.0 {
        dx = -dx;
    }
    let u = BIG + x.abs();
    let x = x.abs() - (u - BIG) + dx;
    let xx = x * x;
    let s = (x * xx).mul_add(xx.mul_add(SN5, SN3), x);
    let c = xx * xx.mul_add(xx.mul_add(CS6, CS4), CS2);
    let (sn, ssn, cs, ccs) = table(u);
    let cor = (-sn).mul_add(s, (-cs).mul_add(c, (-s).mul_add(ssn, ccs)));
    cs + cor
}

/// The same for the sine.
fn do_sin(x: f64, mut dx: f64) -> f64 {
    let xold = x;
    // Max ULP is 0.501 if |x| < 0.126, otherwise ULP is 0.518.
    if x.abs() < 0.126 {
        return taylor_sin(x * x, x, dx);
    }
    if x <= 0.0 {
        dx = -dx;
    }
    let u = BIG + x.abs();
    let x = x.abs() - (u - BIG);
    let xx = x * x;
    let s = x + (x * xx).mul_add(xx.mul_add(SN5, SN3), dx);
    let c = x.mul_add(dx, xx * xx.mul_add(xx.mul_add(CS6, CS4), CS2));
    let (sn, ssn, cs, ccs) = table(u);
    let cor = s.mul_add(cs, (-sn).mul_add(c, s.mul_add(ccs, ssn)));
    (sn + cor).copysign(xold)
}

/// Reduces x to within pi/2 for |x| < 105414350: (n mod 4, the high part, the low part). Accurate to 136 bits, so
/// that when x is large and the result very close to zero, all 53 bits are correct. `xn * pp3` and `xn * pp4`
/// each feed two subtractions, and are fused into both.
fn reduce_sincos(x: f64) -> (i32, f64, f64) {
    let t = x.mul_add(HPINV, TOINT);
    let xn = t - TOINT;
    let y = (-xn).mul_add(MP2, (-xn).mul_add(MP1, x));
    let n = lo(t) & 3;
    let t2 = (-xn).mul_add(PP3, y);
    let mut db = (-xn).mul_add(PP3, y - t2);
    let b = (-xn).mul_add(PP4, t2);
    db += (-xn).mul_add(PP4, t2 - b);
    (n, b, db)
}

/// sin or cos (a + da) for the quadrant n.
fn do_sincos(a: f64, da: f64, n: i32) -> f64 {
    let r = if n & 1 != 0 {
        do_cos(a, da)
    } else {
        do_sin(a, da)
    };
    if n & 2 != 0 {
        -r
    } else {
        r
    }
}

fn ibm_sin(x: f64) -> f64 {
    let k = hi(x) & 0x7fffffff;
    if k < 0x3e500000 {
        // x -> 0 => sin(x) = x
        x
    } else if k < 0x3feb6000 {
        // 2^-26 < |x| < 0.855469
        do_sin(x, 0.0)
    } else if k < 0x400368fd {
        // 0.855469 < |x| < 2.426265
        do_cos(HP0 - x.abs(), HP1).copysign(x)
    } else if k < 0x419921fb {
        // 2.426265 < |x| < 105414350
        let (n, a, da) = reduce_sincos(x);
        do_sincos(a, da, n)
    } else if k < 0x7ff00000 {
        // 105414350 < |x| < 2^1024
        let (n, a, da) = branred(x);
        do_sincos(a, da, n)
    } else {
        // |x| = inf or NaN: NaN, as C computes it.
        #[allow(clippy::eq_op)]
        let nan = x / x;
        nan
    }
}

fn ibm_cos(x: f64) -> f64 {
    let k = hi(x) & 0x7fffffff;
    if k < 0x3e400000 {
        // |x| < 2^-27 => cos(x) = 1
        1.0
    } else if k < 0x3feb6000 {
        do_cos(x, 0.0)
    } else if k < 0x400368fd {
        // Range reduction uses 106 bits here which is sufficient.
        let y = HP0 - x.abs();
        let a = y + HP1;
        let da = (y - a) + HP1;
        do_sin(a, da)
    } else if k < 0x419921fb {
        let (n, a, da) = reduce_sincos(x);
        do_sincos(a, da, n + 1)
    } else if k < 0x7ff00000 {
        let (n, a, da) = branred(x);
        do_sincos(a, da, n + 1)
    } else {
        // |x| = inf or NaN: NaN, as C computes it.
        #[allow(clippy::eq_op)]
        let nan = x / x;
        nan
    }
}

/// 2/pi in base 2^24 (branred.h).
const TOVERP: [f64; 75] = [
    10680707.0, 7228996.0, 1387004.0, 2578385.0, 16069853.0, 12639074.0, 9804092.0, 4427841.0,
    16666979.0, 11263675.0, 12935607.0, 2387514.0, 4345298.0, 14681673.0, 3074569.0, 13734428.0,
    16653803.0, 1880361.0, 10960616.0, 8533493.0, 3062596.0, 8710556.0, 7349940.0, 6258241.0,
    3772886.0, 3769171.0, 3798172.0, 8675211.0, 12450088.0, 3874808.0, 9961438.0, 366607.0,
    15675153.0, 9132554.0, 7151469.0, 3571407.0, 2607881.0, 12013382.0, 4155038.0, 6285869.0,
    7677882.0, 13102053.0, 15825725.0, 473591.0, 9065106.0, 15363067.0, 6271263.0, 9264392.0,
    5636912.0, 4652155.0, 7056368.0, 13614112.0, 10155062.0, 1944035.0, 9527646.0, 15080200.0,
    6658437.0, 6231200.0, 6832269.0, 16767104.0, 5075751.0, 3212806.0, 1398474.0, 7579849.0,
    6349435.0, 12618859.0, 4703257.0, 12806093.0, 14477321.0, 2786137.0, 12875403.0, 9837734.0,
    14528324.0, 13719321.0, 343717.0,
];

/// `__branred`: reduces x into a double-length number a + aa with x = n*pi/2 + (a + aa), |a + aa| < pi/4, for
/// |x| >= 105414350. Returns (n mod 4, a, aa).
fn branred(x: f64) -> (i32, f64, f64) {
    const T576: f64 = f(0x63f0000000000000);
    const TM600: f64 = f(0x1a70000000000000);
    const TM24: f64 = f(0x3e70000000000000);
    const BIG: f64 = f(0x4338000000000000);
    const BIG1: f64 = f(0x4358000000000000);
    const MP2: f64 = f(0xbe4dde9740000000);
    const SPLIT: f64 = 134217729.0; // 2^27 + 1

    // One half of x times 2/pi, in pieces: (the integer part mod 4, b, bb).
    let part = |x: f64| -> (f64, f64, f64) {
        let mut sum = 0.0;
        let k = ((((hi(x) >> 20) & 2047) - 450) / 24).max(0);
        let mut gor = f64::from_bits(T576.to_bits() - (((k * 24) as u64) << 52));
        let mut r = [0.0; 6];
        for (i, ri) in r.iter_mut().enumerate() {
            *ri = x * TOVERP[k as usize + i] * gor;
            gor *= TM24;
        }
        for ri in r.iter_mut().take(3) {
            let s = (*ri + BIG) - BIG;
            sum += s;
            *ri -= s;
        }
        let mut t = 0.0;
        for i in 0..6 {
            t += r[5 - i];
        }
        let bb = (((((r[0] - t) + r[1]) + r[2]) + r[3]) + r[4]) + r[5];
        let s = (t + BIG) - BIG;
        sum += s;
        t -= s;
        let b = t + bb;
        let bb = (t - b) + bb;
        let s = (sum + BIG1) - BIG1;
        sum -= s;
        (sum, b, bb)
    };

    let x = x * TM600;
    let t = x * SPLIT; // split x to two numbers
    let x1 = t - (t - x);
    let x2 = x - x1;
    let (sum1, b1, bb1) = part(x1);
    let (sum2, b2, bb2) = part(x2);

    let mut sum = sum1 + sum2;
    let mut b = b1 + b2;
    let bb = if b1.abs() > b2.abs() {
        (b1 - b) + b2
    } else {
        (b2 - b) + b1
    };
    if b > 0.5 {
        b -= 1.0;
        sum += 1.0;
    } else if b < -0.5 {
        b += 1.0;
        sum -= 1.0;
    }
    let s = b + (bb + bb1 + bb2);
    let t = ((b - s) + bb) + (bb1 + bb2);
    let b = s * SPLIT;
    let t1 = b - (b - s);
    let t2 = s - t1;
    let b = s * HP0;
    let bb = (((t1 * MP1 - b) + t1 * MP2) + t2 * MP1) + (t2 * MP2 + s * HP1 + t * HP0);
    let s = b + bb;
    let t = (b - s) + bb;
    ((sum as i32) & 3, s, t)
}

// atnat2.h
const D3: f64 = f(0xbfd5555555555555);
const D5: f64 = f(0x3fc99999999997fd);
const D7: f64 = f(0xbfc24924923f7603);
const D9: f64 = f(0x3fbc71c6e5129a3b);
const D11: f64 = f(0xbfb7458022b13c25);
const D13: f64 = f(0x3fb375f08b31cbce);
const INV16: f64 = f(0x3fb0000000000000);
const OPI: f64 = f(0x400921fb54442d18);
const OPI1: f64 = f(0x3ca1a62633145c07);
const MOPI: f64 = f(0xc00921fb54442d18);
const HPI: f64 = f(0x3ff921fb54442d18);
const HPI1: f64 = f(0x3c91a62633145c07);
const MHPI: f64 = f(0xbff921fb54442d18);
const QPI: f64 = f(0x3fe921fb54442d18);
const MQPI: f64 = f(0xbfe921fb54442d18);
const TQPI: f64 = f(0x4002d97c7f3321d2);
const MTQPI: f64 = f(0xc002d97c7f3321d2);
const TWO500: f64 = f(0x5f30000000000000);
const TWOM500: f64 = f(0x20b0000000000000);
const TWO52: f64 = f(0x4330000000000000);

fn cij(i: usize, j: usize) -> f64 {
    f64::from_bits(CIJ[i * 7 + j])
}

/// The first polynomial: d3 + v*(d5 + v*(d7 + v*(d9 + v*(d11 + v*d13)))).
fn poly_d(v: f64) -> f64 {
    v.mul_add(
        v.mul_add(v.mul_add(v.mul_add(v.mul_add(D13, D11), D9), D7), D5),
        D3,
    )
}

/// c2 + v*(c3 + v*(c4 + v*(c5 + v*c6))) of table row i, from coefficient `from`.
fn poly_c(i: usize, from: usize, v: f64) -> f64 {
    let mut p = cij(i, 6);
    for j in (from..6).rev() {
        p = v.mul_add(p, cij(i, j));
    }
    p
}

/// The table row for u in (1/16, 1): `(TWO52 + 256 * u) - TWO52 - 16`.
fn row(u: f64) -> usize {
    (((TWO52 + 256.0 * u) - TWO52) as i32 - 16) as usize
}

/// `EADD`: z + zz = x + y exactly.
fn eadd(x: f64, y: f64) -> (f64, f64) {
    let z = x + y;
    let zz = if x.abs() > y.abs() {
        (x - z) + y
    } else {
        (y - z) + x
    };
    (z, zz)
}

/// `ESUB`: z + zz = x - y exactly.
fn esub(x: f64, y: f64) -> (f64, f64) {
    let z = x - y;
    let zz = if x.abs() > y.abs() {
        (x - z) - y
    } else {
        x - (y + z)
    };
    (z, zz)
}

/// `__ieee754_atan2`, max ULP of ~0.524 based on random sampling.
fn ibm_atan2(y: f64, x: f64) -> f64 {
    let (ux, dx) = (hi(x), lo(x));
    let (uy, dy) = (hi(y), lo(y));
    // x=NaN or y=NaN
    if x.is_nan() {
        return x + y;
    }
    if y.is_nan() {
        return y + y;
    }
    let x_negative = ux < 0;
    // y=+-0
    if uy == 0 && dy == 0 {
        return if x_negative { OPI } else { 0.0 };
    }
    if uy == i32::MIN && dy == 0 {
        return if x_negative { MOPI } else { -0.0 };
    }
    // x=+-0
    if x == 0.0 {
        return if uy >= 0 { HPI } else { MHPI };
    }
    // x=+-INF
    if x.is_infinite() {
        let _ = dx;
        return match (y.is_infinite(), x > 0.0, y > 0.0) {
            (true, true, true) => QPI,
            (true, true, false) => MQPI,
            (true, false, true) => TQPI,
            (true, false, false) => MTQPI,
            (false, true, positive) => {
                if positive {
                    0.0
                } else {
                    -0.0
                }
            }
            (false, false, positive) => {
                if positive {
                    OPI
                } else {
                    MOPI
                }
            }
        };
    }
    // y=+-INF
    if y.is_infinite() {
        return if y > 0.0 { HPI } else { MHPI };
    }

    // Either x/y or y/x is very close to zero.
    let mut ax = x.abs();
    let mut ay = y.abs();
    let de = (uy & 0x7ff00000) - (ux & 0x7ff00000);
    if de >= 59768832 {
        // 57*16**5
        return if y > 0.0 { HPI } else { MHPI };
    } else if de <= -59768832 {
        if x > 0.0 {
            return (ay / ax).copysign(y);
        }
        return if y > 0.0 { OPI } else { MOPI };
    }

    // If either x or y is extremely close to zero, scale abs(x), abs(y).
    if ax < TWOM500 || ay < TWOM500 {
        ax *= TWO500;
        ay *= TWO500;
    }
    // Likewise for large x and y.
    if ax > TWO500 || ay > TWO500 {
        ax *= TWOM500;
        ay *= TWOM500;
    }

    // x,y which are neither special nor extreme. `EMULV` with FMA: v + vv = ax * u exactly.
    let (u, du) = if ay < ax {
        let u = ay / ax;
        let v = ax * u;
        let vv = ax.mul_add(u, -v);
        (u, ((ay - v) - vv) / ax)
    } else {
        let u = ax / ay;
        let v = ay * u;
        let vv = ay.mul_add(u, -v);
        (u, ((ax - v) - vv) / ay)
    };

    if x > 0.0 {
        // (i) x>0, abs(y)< abs(x): atan(ay/ax)
        if ay < ax {
            if u < INV16 {
                let v = u * u;
                let zz = (u * v).mul_add(poly_d(v), du);
                // Max ULP is 0.504.
                return (u + zz).copysign(y);
            }
            let i = row(u);
            let t3 = u - cij(i, 0);
            let (v, dv) = eadd(t3, du);
            let t1 = cij(i, 1);
            let t2 = cij(i, 2);
            let zz = v.mul_add(t2, dv.mul_add(t2, (v * v) * poly_c(i, 3, v)));
            // Max ULP is 0.56.
            return (t1 + zz).copysign(y);
        }
        // (ii) x>0, abs(x)<=abs(y): pi/2-atan(ax/ay)
        if u < INV16 {
            let v = u * u;
            // Not fused into t3: ESUB's branch puts the product in another basic block than its use.
            let zz = u * v * poly_d(v);
            let (t2, cor) = esub(HPI, u);
            let t3 = ((HPI1 + cor) - du) - zz;
            // Max ULP is 0.501.
            return (t2 + t3).copysign(y);
        }
        let i = row(u);
        let v = (u - cij(i, 0)) + du;
        let zz = (-v).mul_add(poly_c(i, 2, v), HPI1);
        let t1 = HPI - cij(i, 1);
        // Max ULP is 0.503.
        return (t1 + zz).copysign(y);
    }

    // (iii) x<0, abs(x)< abs(y): pi/2+atan(ax/ay)
    if ax < ay {
        if u < INV16 {
            let v = u * u;
            let zz = u * v * poly_d(v);
            let (t2, cor) = eadd(HPI, u);
            let t3 = ((HPI1 + cor) + du) + zz;
            // Max ULP is 0.501.
            return (t2 + t3).copysign(y);
        }
        let i = row(u);
        let v = (u - cij(i, 0)) + du;
        let zz = v.mul_add(poly_c(i, 2, v), HPI1);
        let t1 = HPI + cij(i, 1);
        // Max ULP is 0.503.
        return (t1 + zz).copysign(y);
    }

    // (iv) x<0, abs(y)<=abs(x): pi-atan(ax/ay)
    if u < INV16 {
        let v = u * u;
        let zz = u * v * poly_d(v);
        let (t2, cor) = esub(OPI, u);
        let t3 = ((OPI1 + cor) - du) - zz;
        // Max ULP is 0.501.
        return (t2 + t3).copysign(y);
    }
    let i = row(u);
    let v = (u - cij(i, 0)) + du;
    let zz = (-v).mul_add(poly_c(i, 2, v), OPI1);
    let t1 = OPI - cij(i, 1);
    // Max ULP is 0.502.
    (t1 + zz).copysign(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        /// A double of uniformly random sign and exponent in [lo, hi] (powers of two).
        fn scaled(&mut self, lo: i32, hi: i32) -> f64 {
            let m = 1.0 + (self.next() >> 12) as f64 * 2f64.powi(-52);
            let e = lo + (self.next() % (hi - lo + 1) as u64) as i32;
            let s = if self.next() & 1 == 0 { 1.0 } else { -1.0 };
            s * m * 2f64.powi(e)
        }
    }

    fn same(what: &str, args: &[f64], want: f64, got: f64) {
        assert!(
            want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan()),
            "{what}{args:?}: glibc {want:e}, port {got:e}"
        );
    }

    const SPECIALS: [f64; 12] = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        5e-324,
        f64::MAX,
        f64::MIN_POSITIVE,
        std::f64::consts::PI,
        1e300,
    ];

    /// sin and cos against glibc on every range of the argument (Taylor, table, the two reductions).
    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn matches_platform_sin_cos() {
        let check = |x: f64| {
            let x = std::hint::black_box(x);
            same("sin", &[x], x.sin(), ibm_sin(x));
            same("cos", &[x], x.cos(), ibm_cos(x));
        };
        for x in SPECIALS {
            check(x);
        }
        for i in -2000..2000 {
            check(i as f64 / 64.0);
            check(i as f64 * std::f64::consts::FRAC_PI_4);
        }
        let mut rng = Rng(0x9e3779b97f4a7c15);
        for _ in 0..300_000 {
            check(rng.scaled(-30, 2));
            check(rng.scaled(1, 27));
            check(rng.scaled(26, 1023));
            check(f64::from_bits(rng.next()));
        }
    }

    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn matches_platform_atan2() {
        let check = |y: f64, x: f64| {
            let (y, x) = (std::hint::black_box(y), std::hint::black_box(x));
            same("atan2", &[y, x], y.atan2(x), ibm_atan2(y, x));
        };
        for y in SPECIALS {
            for x in SPECIALS {
                check(y, x);
            }
        }
        for a in -40..40 {
            for b in -40..40 {
                check(a as f64 / 4.0, b as f64 / 3.0);
            }
        }
        let mut rng = Rng(0x2545f4914f6cdd1d);
        for _ in 0..400_000 {
            check(rng.scaled(-8, 8), rng.scaled(-8, 8));
            check(rng.scaled(-1074, 1023), rng.scaled(-1074, 1023));
            check(f64::from_bits(rng.next()), f64::from_bits(rng.next()));
        }
    }

    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn matches_platform_exp() {
        let check = |x: f64| {
            let x = std::hint::black_box(x);
            same("exp", &[x], x.exp(), pow::arm_exp(x));
        };
        for x in SPECIALS {
            check(x);
            check(-x);
        }
        for x in [709.78, 709.79, -708.4, -745.1, -745.2, 1e-20, -1e-20] {
            check(x);
        }
        let mut rng = Rng(0x853c49e6748fea9b);
        for _ in 0..1_000_000 {
            check(rng.scaled(-60, 10));
            check(f64::from_bits(rng.next()));
        }
    }
}
