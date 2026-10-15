//! `log(double)` exactly as glibc computes it, for targets whose libm differs (WASM). See `pow.rs` for why.
//!
//! CPython's `math.log` ends in the platform `log` (`m_log`). On the oracle's Linux that is glibc's, which since
//! 2.28 is Arm's optimized-routines algorithm (worst case 0.519 ULP, so not always correctly rounded). Natively the
//! port calls glibc through `f64::ln`; on wasm32 this line-by-line port of Arm's log.c (as in musl's
//! src/math/log.c, MIT, Copyright (c) 2018 Arm Limited) stands in. It takes the `__FP_FAST_FMA` paths and the
//! contractions of glibc 2.43's x86_64 `__log_fma` (read off its disassembly; each fused `a * b + c` is a
//! `mul_add` marked "contracted"). The tables are glibc's `__log_data`, read out of its libm.so.
//! `tests::matches_platform_log` checks the port against glibc natively.
//!
//! Also `hypot` (for `abs(complex)` and complex powers): glibc's own since 2.35, ported below the same way and
//! checked by `tests::matches_platform_hypot`.

/// The natural logarithm with the platform's rounding: glibc's `log` (natively) or its port below (wasm32).
pub fn log(x: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        arm_log(x)
    } else {
        x.ln()
    }
}

/// `hypot(x, y)` with the platform's rounding: glibc's (natively) or its port below (wasm32).
pub fn hypot(x: f64, y: f64) -> f64 {
    if cfg!(target_arch = "wasm32") {
        glibc_hypot(x, y)
    } else {
        x.hypot(y)
    }
}

const HYPOT_SCALE: f64 = f64::from_bits(0x1a70000000000000); // 0x1p-600
const HYPOT_LARGE_VAL: f64 = f64::from_bits(0x5fe0000000000000); // 0x1p+511
const HYPOT_TINY_VAL: f64 = f64::from_bits(0x2340000000000000); // 0x1p-459
const HYPOT_EPS: f64 = f64::from_bits(0x3c90000000000000); // 0x1p-54

/// glibc's hypot kernel (sysdeps/ieee754/dbl-64/e_hypot.c, after Borges, "An Improved Algorithm for
/// hypot(a,b)"), its build without `__FP_FAST_FMA` (x86_64 has no FMA variant of hypot; the constants and the
/// operation order were checked against the disassembly of glibc 2.43's `__hypot`). The inputs must be adjusted so that ax >= ay >= 0 and squaring
/// ax, ay and (ax - ay) does not overflow or underflow.
fn hypot_kernel(ax: f64, ay: f64) -> f64 {
    let mut h = (ax * ax + ay * ay).sqrt();
    let (t1, t2);
    if h <= 2.0 * ay {
        let delta = h - ay;
        t1 = ax * (2.0 * delta - ax);
        t2 = (delta - 2.0 * (ax - ay)) * delta;
    } else {
        let delta = h - ax;
        t1 = 2.0 * delta * (ax - 2.0 * ay);
        t2 = (4.0 * delta - ay) * ay + delta * delta;
    }
    h -= (t1 + t2) / (2.0 * h);
    h
}

/// glibc's `__hypot`.
fn glibc_hypot(x: f64, y: f64) -> f64 {
    if !x.is_finite() || !y.is_finite() {
        // (Signaling NaNs aside: Rust has none to give.)
        if x.is_infinite() || y.is_infinite() {
            return f64::INFINITY;
        }
        return x + y;
    }
    let (x, y) = (x.abs(), y.abs());
    let ax = if x < y { y } else { x };
    let ay = if x < y { x } else { y };
    // If ax is huge, scale both inputs down.
    if ax > HYPOT_LARGE_VAL {
        if ay <= ax * HYPOT_EPS {
            return ax + ay;
        }
        return hypot_kernel(ax * HYPOT_SCALE, ay * HYPOT_SCALE) / HYPOT_SCALE;
    }
    // If ay is tiny, scale both inputs up to avoid underflow.
    if ay < HYPOT_TINY_VAL {
        if ax >= ay / HYPOT_EPS {
            return ax + ay;
        }
        return hypot_kernel(ax / HYPOT_SCALE, ay / HYPOT_SCALE) * HYPOT_SCALE;
    }
    // Common case: ax is not huge and ay is not tiny.
    if ay <= ax * HYPOT_EPS {
        return ax + ay;
    }
    hypot_kernel(ax, ay)
}

const LOG_TABLE_BITS: u32 = 7;
const N: u64 = 1 << LOG_TABLE_BITS;
const OFF: u64 = 0x3fe6000000000000;

const LN2HI: f64 = f64::from_bits(0x3fe62e42fefa3800); // 0x1.62e42fefa3800p-1
const LN2LO: f64 = f64::from_bits(0x3d2ef35793c76730); // 0x1.ef35793c76730p-45
/// Polynomial for log1p(r) - r around the table points, A[0] ~ -0.5. relative error: 0x1.926199e8p-56,
/// abs error: 0x1.882ff33p-65, in -0x1.fp-9 0x1.fp-9.
const A: [u64; 5] = [
    0xbfe0000000000001,
    0x3fd555555551305b,
    0xbfcfffffffeb4590,
    0x3fc999b324f10111,
    0xbfc55575e506c89f,
];
/// Polynomial for log1p(r) near 1, B[0] = -0.5. relative error: 0x1.c04d76cp-63 in -0x1p-4 0x1.09p-4 (|log(1+x)|
/// > 0x1p-4 outside the interval).
const B: [u64; 11] = [
    0xbfe0000000000000,
    0x3fd5555555555577,
    0xbfcffffffffffdcb,
    0x3fc999999995dd0c,
    0xbfc55555556745a7,
    0x3fc24924a344de30,
    0xbfbfffffa4423d65,
    0x3fbc7184282ad6ca,
    0xbfb999eb43b068ff,
    0x3fb78182f7afd085,
    0xbfb5521375d145cd,
];

fn a(i: usize) -> f64 {
    f64::from_bits(A[i])
}

fn b(i: usize) -> f64 {
    f64::from_bits(B[i])
}

/// Arm's `log`, glibc's FMA build.
fn arm_log(x: f64) -> f64 {
    let mut ix = x.to_bits();
    const LO: u64 = 0x3fee000000000000; // asuint64(1.0 - 0x1p-4)
    const HI: u64 = 0x3ff1090000000000; // asuint64(1.0 + 0x1.09p-4)
    if ix.wrapping_sub(LO) < HI - LO {
        // Handle close to 1.0 inputs separately.
        // Fix sign of zero with downward rounding when x==1.
        if ix == 0x3ff0000000000000 {
            return 0.0;
        }
        let r = x - 1.0;
        let r2 = r * r;
        let r3 = r * r2;
        // contracted, as glibc's build evaluates the polynomial.
        let p = r2.mul_add(b(3), r.mul_add(b(2), b(1)));
        let q = r2.mul_add(b(6), r.mul_add(b(5), b(4)));
        let s = r3.mul_add(b(10), r2.mul_add(b(9), r.mul_add(b(8), b(7))));
        let poly = s.mul_add(r3, q).mul_add(r3, p);
        // Worst-case error is around 0.507 ULP.
        // w = r * 0x1p27; rhi = r + w - w (both contracted).
        let rw = r.mul_add(134217728.0, r);
        let rhi = (-134217728.0f64).mul_add(r, rw);
        let rlo = r - rhi;
        let rhi2 = rhi * rhi;
        // w = rhi * rhi * B[0], B[0] == -0.5; hi = r + w; lo = r - hi + w (contracted).
        let hi = rhi2.mul_add(b(0), r);
        let lo = rhi2.mul_add(b(0), r - hi);
        // lo += B[0] * rlo * (rhi + r) (contracted).
        let lo = (b(0) * rlo).mul_add(rhi + r, lo);
        // y = r3 * poly; y += lo (contracted); y += hi.
        return poly.mul_add(r3, lo) + hi;
    }
    let top = (ix >> 48) as u32;
    if top.wrapping_sub(0x0010) >= 0x7ff0 - 0x0010 {
        // x < 0x1p-1022 or inf or nan.
        if ix.wrapping_mul(2) == 0 {
            return f64::NEG_INFINITY; // __math_divzero (1)
        }
        if ix == f64::INFINITY.to_bits() {
            // log(inf) == inf.
            return x;
        }
        if (top & 0x8000) != 0 || (top & 0x7ff0) == 0x7ff0 {
            // __math_invalid (x): a NaN, keeping the payload of a NaN argument.
            #[allow(clippy::eq_op)]
            return (x - x) / (x - x);
        }
        // x is subnormal, normalize it.
        ix = (x * 4503599627370496.0).to_bits(); // 0x1p52
        ix = ix.wrapping_sub(52 << 52);
    }
    // x = 2^k z; where z is in range [OFF,2*OFF) and exact. The range is split into N subintervals. The ith
    // subinterval contains z and c is near its center.
    let tmp = ix.wrapping_sub(OFF);
    let i = ((tmp >> (52 - LOG_TABLE_BITS)) % N) as usize;
    let k = (tmp as i64 >> 52) as i32; // arithmetic shift
    let iz = ix.wrapping_sub(tmp & (0xfff << 52));
    let [invc, logc] = LOG_TAB[i].map(f64::from_bits);
    let z = f64::from_bits(iz);
    // log(x) = log1p(z/c-1) + log(c) + k*Ln2.
    // r ~= z/c - 1, |r| < 1/(2*N); rounding error: 0x1p-55/N.
    let r = z.mul_add(invc, -1.0);
    let kd = k as f64;
    // hi + lo = r + log(c) + k*Ln2.
    let w = kd.mul_add(LN2HI, logc); // contracted
    let hi = w + r;
    let lo = kd.mul_add(LN2LO, w - hi + r); // contracted
                                            // log(x) = lo + (log1p(r) - r) + hi.
    let r2 = r * r; // rounding error: 0x1p-54/N^2.
                    // Worst case error if |y| > 0x1p-5: 0.5 + 4.13/N + abs-poly-error*2^57 ULP (+ 0.002 ULP without fma).
                    // Worst case error if |y| > 0x1p-4: 0.5 + 2.06/N + abs-poly-error*2^56 ULP (+ 0.001 ULP without fma).
                    // y = lo + r2 * A[0] + r * r2 * (A[1] + r * A[2] + r2 * (A[3] + r * A[4])) + hi (all contracted).
    let inner = r.mul_add(a(4), a(3)).mul_add(r2, r.mul_add(a(2), a(1)));
    (r * r2).mul_add(inner, r2.mul_add(a(0), lo)) + hi
}

/// `__log_data.tab`: `{invc, logc}` per subinterval. 1/c is close to the center of the subinterval and is chosen
/// so that `z*invc - 1` is exact (with FMA no special care is needed for that), and `logc = round(log(c))`.
const LOG_TAB: [[u64; 2]; 128] = [
    [0x3ff734f0c3e0de9f, 0xbfd7cc7f79e69000],
    [0x3ff713786a2ce91f, 0xbfd76feec20d0000],
    [0x3ff6f26008fab5a0, 0xbfd713e31351e000],
    [0x3ff6d1a61f138c7d, 0xbfd6b85b38287800],
    [0x3ff6b1490bc5b4d1, 0xbfd65d5590807800],
    [0x3ff69147332f0cba, 0xbfd602d076180000],
    [0x3ff6719f18224223, 0xbfd5a8ca86909000],
    [0x3ff6524f99a51ed9, 0xbfd54f4356035000],
    [0x3ff63356aa8f24c4, 0xbfd4f637c36b4000],
    [0x3ff614b36b9ddc14, 0xbfd49da7fda85000],
    [0x3ff5f66452c65c4c, 0xbfd445923989a800],
    [0x3ff5d867b5912c4f, 0xbfd3edf439b0b800],
    [0x3ff5babccb5b90de, 0xbfd396ce448f7000],
    [0x3ff59d61f2d91a78, 0xbfd3401e17bda000],
    [0x3ff5805612465687, 0xbfd2e9e2ef468000],
    [0x3ff56397cee76bd3, 0xbfd2941b3830e000],
    [0x3ff54725e2a77f93, 0xbfd23ec58cda8800],
    [0x3ff52aff42064583, 0xbfd1e9e129279000],
    [0x3ff50f22dbb2bddf, 0xbfd1956d2b48f800],
    [0x3ff4f38f4734ded7, 0xbfd141679ab9f800],
    [0x3ff4d843cfde2840, 0xbfd0edd094ef9800],
    [0x3ff4bd3ec078a3c8, 0xbfd09aa518db1000],
    [0x3ff4a27fc3e0258a, 0xbfd047e65263b800],
    [0x3ff4880524d48434, 0xbfcfeb224586f000],
    [0x3ff46dce1b192d0b, 0xbfcf474a7517b000],
    [0x3ff453d9d3391854, 0xbfcea4443d103000],
    [0x3ff43a2744b4845a, 0xbfce020d44e9b000],
    [0x3ff420b54115f8fb, 0xbfcd60a22977f000],
    [0x3ff40782da3ef4b1, 0xbfccc00104959000],
    [0x3ff3ee8f5d57fe8f, 0xbfcc202956891000],
    [0x3ff3d5d9a00b4ce9, 0xbfcb81178d811000],
    [0x3ff3bd60c010c12b, 0xbfcae2c9ccd3d000],
    [0x3ff3a5242b75dab8, 0xbfca45402e129000],
    [0x3ff38d22cd9fd002, 0xbfc9a877681df000],
    [0x3ff3755bc5847a1c, 0xbfc90c6d69483000],
    [0x3ff35dce49ad36e2, 0xbfc87120a645c000],
    [0x3ff34679984dd440, 0xbfc7d68fb4143000],
    [0x3ff32f5cceffcb24, 0xbfc73cb83c627000],
    [0x3ff3187775a10d49, 0xbfc6a39a9b376000],
    [0x3ff301c8373e3990, 0xbfc60b3154b7a000],
    [0x3ff2eb4ebb95f841, 0xbfc5737d76243000],
    [0x3ff2d50a0219a9d1, 0xbfc4dc7b8fc23000],
    [0x3ff2bef9a8b7fd2a, 0xbfc4462c51d20000],
    [0x3ff2a91c7a0c1bab, 0xbfc3b08abc830000],
    [0x3ff293726014b530, 0xbfc31b996b490000],
    [0x3ff27dfa5757a1f5, 0xbfc2875490a44000],
    [0x3ff268b39b1d3bbf, 0xbfc1f3b9f879a000],
    [0x3ff2539d838ff5bd, 0xbfc160c8252ca000],
    [0x3ff23eb7aac9083b, 0xbfc0ce7f57f72000],
    [0x3ff22a012ba940b6, 0xbfc03cdc49fea000],
    [0x3ff2157996cc4132, 0xbfbf57bdbc4b8000],
    [0x3ff201201dd2fc9b, 0xbfbe370896404000],
    [0x3ff1ecf4494d480b, 0xbfbd17983ef94000],
    [0x3ff1d8f5528f6569, 0xbfbbf9674ed8a000],
    [0x3ff1c52311577e7c, 0xbfbadc79202f6000],
    [0x3ff1b17c74cb26e9, 0xbfb9c0c3e7288000],
    [0x3ff19e010c2c1ab6, 0xbfb8a646b372c000],
    [0x3ff18ab07bb670bd, 0xbfb78d01b3ac0000],
    [0x3ff1778a25efbcb6, 0xbfb674f145380000],
    [0x3ff1648d354c31da, 0xbfb55e0e6d878000],
    [0x3ff151b990275fdd, 0xbfb4485cdea1e000],
    [0x3ff13f0ea432d24c, 0xbfb333d94d6aa000],
    [0x3ff12c8b7210f9da, 0xbfb22079f8c56000],
    [0x3ff11a3028ecb531, 0xbfb10e4698622000],
    [0x3ff107fbda8434af, 0xbfaffa6c6ad20000],
    [0x3ff0f5ee0f4e6bb3, 0xbfadda8d4a774000],
    [0x3ff0e4065d2a9fce, 0xbfabbcece4850000],
    [0x3ff0d244632ca521, 0xbfa9a1894012c000],
    [0x3ff0c0a77ce2981a, 0xbfa788583302c000],
    [0x3ff0af2f83c636d1, 0xbfa5715e67d68000],
    [0x3ff09ddb98a01339, 0xbfa35c8a49658000],
    [0x3ff08cabaf52e7df, 0xbfa149e364154000],
    [0x3ff07b9f2f4e28fb, 0xbf9e72c082eb8000],
    [0x3ff06ab58c358f19, 0xbf9a55f152528000],
    [0x3ff059eea5ecf92c, 0xbf963d62cf818000],
    [0x3ff04949cdd12c90, 0xbf9228fb8caa0000],
    [0x3ff038c6c6f0ada9, 0xbf8c317b20f90000],
    [0x3ff02865137932a9, 0xbf8419355daa0000],
    [0x3ff0182427ea7348, 0xbf781203c2ec0000],
    [0x3ff008040614b195, 0xbf60040979240000],
    [0x3fefe01ff726fa1a, 0x3f6feff384900000],
    [0x3fefa11cc261ea74, 0x3f87dc41353d0000],
    [0x3fef6310b081992e, 0x3f93cea3c4c28000],
    [0x3fef25f63ceeadcd, 0x3f9b9fc114890000],
    [0x3feee9c8039113e7, 0x3fa1b0d8ce110000],
    [0x3feeae8078cbb1ab, 0x3fa58a5bd001c000],
    [0x3fee741aa29d0c9b, 0x3fa95c8340d88000],
    [0x3fee3a91830a99b5, 0x3fad276aef578000],
    [0x3fee01e009609a56, 0x3fb07598e598c000],
    [0x3fedca01e577bb98, 0x3fb253f5e30d2000],
    [0x3fed92f20b7c9103, 0x3fb42edd8b380000],
    [0x3fed5cac66fb5cce, 0x3fb606598757c000],
    [0x3fed272caa5ede9d, 0x3fb7da76356a0000],
    [0x3fecf26e3e6b2ccd, 0x3fb9ab434e1c6000],
    [0x3fecbe6da2a77902, 0x3fbb78c7bb0d6000],
    [0x3fec8b266d37086d, 0x3fbd431332e72000],
    [0x3fec5894bd5d5804, 0x3fbf0a3171de6000],
    [0x3fec26b533bb9f8c, 0x3fc067152b914000],
    [0x3febf583eeece73f, 0x3fc147858292b000],
    [0x3febc4fd75db96c1, 0x3fc2266ecdca3000],
    [0x3feb951e0c864a28, 0x3fc303d7a6c55000],
    [0x3feb65e2c5ef3e2c, 0x3fc3dfc33c331000],
    [0x3feb374867c9888b, 0x3fc4ba366b7a8000],
    [0x3feb094b211d304a, 0x3fc5933928d1f000],
    [0x3feadbe885f2ef7e, 0x3fc66acd2418f000],
    [0x3feaaf1d31603da2, 0x3fc740f8ec669000],
    [0x3fea82e63fd358a7, 0x3fc815c0f51af000],
    [0x3fea5740ef09738b, 0x3fc8e92954f68000],
    [0x3fea2c2a90ab4b27, 0x3fc9bb3602f84000],
    [0x3fea01a01393f2d1, 0x3fca8bed1c2c0000],
    [0x3fe9d79f24db3c1b, 0x3fcb5b515c01d000],
    [0x3fe9ae2505c7b190, 0x3fcc2967ccbcc000],
    [0x3fe9852ef297ce2f, 0x3fccf635d5486000],
    [0x3fe95cbaeea44b75, 0x3fcdc1bd3446c000],
    [0x3fe934c69de74838, 0x3fce8c01b8cfe000],
    [0x3fe90d4f2f6752e6, 0x3fcf5509c0179000],
    [0x3fe8e6528effd79d, 0x3fd00e6c121fb800],
    [0x3fe8bfce9fcc007c, 0x3fd071b80e93d000],
    [0x3fe899c0dabec30e, 0x3fd0d46b9e867000],
    [0x3fe87427aa2317fb, 0x3fd13687334bd000],
    [0x3fe84f00acb39a08, 0x3fd1980d67234800],
    [0x3fe82a49e8653e55, 0x3fd1f8ffe0cc8000],
    [0x3fe8060195f40260, 0x3fd2595fd7636800],
    [0x3fe7e22563e0a329, 0x3fd2b9300914a800],
    [0x3fe7beb377dcb5ad, 0x3fd3187210436000],
    [0x3fe79baa679725c2, 0x3fd377266dec1800],
    [0x3fe77907f2170657, 0x3fd3d54ffbaf3000],
    [0x3fe756cadbd6130c, 0x3fd432eee32fe000],
];

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*, as in pow.rs.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545f4914f6cdd1d)
        }
    }

    fn same(x: f64) {
        let x = std::hint::black_box(x);
        let (want, got) = (x.ln(), arm_log(x));
        assert!(
            want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan()),
            "log({x:e}): glibc {want:e}, port {got:e}"
        );
    }

    /// The port against the platform log (glibc on the machines that run the gate): probabilities as `float`s
    /// (ratios of small ints, what `entropy` takes logs of), the region near 1, arbitrary bit patterns, specials.
    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn matches_platform_log() {
        for x in [
            0.0,
            -0.0,
            1.0,
            -1.0,
            2.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            5e-324,
            f64::MAX,
        ] {
            same(x);
        }
        for d in 1..2000u32 {
            for n in 1..=d.min(300) {
                same(n as f64 / d as f64);
            }
        }
        let mut rng = Rng(0x9e3779b97f4a7c15);
        for _ in 0..1_000_000 {
            same(f64::from_bits(rng.next()));
            // Near 1, where the second polynomial takes over.
            same(1.0 + (rng.next() as i64 as f64) * 2f64.powi(-67));
            same(f64::from_bits(rng.next() >> 12)); // subnormal
        }
    }

    fn same_hypot(x: f64, y: f64) {
        let (x, y) = (std::hint::black_box(x), std::hint::black_box(y));
        let (want, got) = (x.hypot(y), glibc_hypot(x, y));
        assert!(
            want.to_bits() == got.to_bits() || (want.is_nan() && got.is_nan()),
            "hypot({x:e}, {y:e}): glibc {want:e}, port {got:e}"
        );
    }

    /// The port against the platform hypot: small ints and their halves (what complex outcomes are made of),
    /// arbitrary bit patterns, nearby magnitudes, huge and tiny scales, specials.
    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn matches_platform_hypot() {
        let specials = [
            0.0,
            -0.0,
            1.0,
            -3.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            5e-324,
            f64::MAX,
        ];
        for x in specials {
            for y in specials {
                same_hypot(x, y);
            }
        }
        for a in -40..40 {
            for b in -40..40 {
                same_hypot(a as f64 / 4.0, b as f64 / 3.0);
            }
        }
        let mut rng = Rng(0x9e3779b97f4a7c15);
        for _ in 0..1_000_000 {
            same_hypot(f64::from_bits(rng.next()), f64::from_bits(rng.next()));
            let x = f64::from_bits(rng.next() >> 2);
            same_hypot(x, x * (1.0 + (rng.next() >> 11) as f64 * 2f64.powi(-53)));
            same_hypot(x, x * f64::from_bits(rng.next() >> 2) * 2f64.powi(-512));
            same_hypot(
                f64::from_bits(rng.next() >> 12),
                f64::from_bits(rng.next() >> 12),
            ); // subnormals
        }
    }
}
