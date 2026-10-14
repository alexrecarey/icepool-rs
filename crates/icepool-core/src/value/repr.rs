//! Python's `repr(float)`: the shortest string that round-trips, switching to exponent notation when the decimal
//! exponent is below -4 or at least 16 (CPython `Python/pystrtod.c`, `format_float_short` with mode `'r'`).

pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // Rust's `{:e}` gives the same shortest round-trip digits, as `d.ddde<exp>`.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = break_tie_to_even(x.abs(), digits, exp);
    let sign = if x.is_sign_negative() { "-" } else { "" };
    // `decpt` is where the decimal point goes relative to the digits: x = 0.<digits> * 10**decpt.
    let decpt = exp + 1;
    let body = if !(-4 < decpt && decpt <= 16) {
        let (first, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let esign = if exp < 0 { '-' } else { '+' };
        format!("{first}{frac}e{esign}{:02}", exp.abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt as usize >= digits.len() {
        format!("{digits}{}.0", "0".repeat(decpt as usize - digits.len()))
    } else {
        let (int, frac) = digits.split_at(decpt as usize);
        format!("{int}.{frac}")
    };
    format!("{sign}{body}")
}

/// Rust and Python agree on the shortest digit count, and on the digits whenever one string of that length is
/// closest to `x`. When two are exactly equally close (`x` = 714285714285714.25 between `...714.2` and `...714.3`),
/// Rust rounds the last digit up, while Python's dtoa takes the even one. `digits` (with the decimal exponent
/// `exp` of its first digit) is Rust's choice for the positive finite `x`; returns Python's.
fn break_tie_to_even(x: f64, digits: String, exp: i32) -> String {
    use dashu_int::IBig;
    use dashu_ratio::RBig;
    let last_is_odd = digits.as_bytes()[digits.len() - 1] % 2 == 1;
    // Only an odd last digit can lose a tie, and only to the next smaller string (Rust rounds up).
    let Ok(d) = digits.parse::<u64>() else {
        return digits;
    };
    if !last_is_odd || d <= 1 {
        return digits;
    }
    let scale_exp = exp - (digits.len() as i32 - 1);
    let candidate = (d - 1).to_string();
    // The candidate must still be shortest (same length) and still read back as x.
    if candidate.len() != digits.len() || format!("{candidate}e{scale_exp}").parse::<f64>() != Ok(x)
    {
        return digits;
    }
    let exact = RBig::try_from(x).expect("finite");
    let pow10 = RBig::from(IBig::from(10u8).pow(scale_exp.unsigned_abs() as usize));
    let value = |n: u64| {
        let n = RBig::from(IBig::from(n));
        if scale_exp >= 0 {
            n * &pow10
        } else {
            n / &pow10
        }
    };
    let up = value(d) - &exact;
    let down = &exact - value(d - 1);
    if up == down {
        candidate
    } else {
        digits
    }
}

#[cfg(test)]
mod tests {
    use super::float_repr;

    #[test]
    fn matches_python_repr() {
        for (x, s) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.5, "1.5"),
            (100.0, "100.0"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1.2345e16, "1.2345e+16"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (0.1 + 0.2, "0.30000000000000004"),
            // Ties between two shortest strings go to the even last digit, as in Python.
            (5e15 / 7.0, "714285714285714.2"),
            (2f64.powi(-25), "2.9802322387695312e-08"),
            (1e300, "1e+300"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::NAN, "nan"),
        ] {
            assert_eq!(float_repr(x), s);
        }
    }
}
