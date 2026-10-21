//! population/format.py: a population as a Markdown, BBCode, HTML or CSV table, and `Population.format`'s spec
//! parsing, for any outcome type that says how it prints ([`FormatOutcome`]).
//!
//! Every cell is what Python's `str()` makes of something: the outcomes through `FormatOutcome` (Python's own
//! `str` for `Value`), the quantities as Python prints ints, the probabilities as `f'{float(x):0.6f}'`. So a typed
//! die prints exactly as the same die of Python ints, bools, strs or tuples does.

use dashu_int::IBig;

use crate::error::{Error, Result};
use crate::host::LenKind;
use crate::outcome::OutcomeOrder;
use crate::population::Comparison;
use crate::value::format::format_probability_inverse;
use crate::value::pystr::{format_float, int_str, str_repr, FloatFormat};
use crate::value::ratio_to_f64;
use crate::outcome::Outcome;
use crate::{Deal, Deck, Die, Int, Pool, Quantity, Ratio, Value};

/// How an outcome prints in icepool's tables: Python's `str()` of it, and the parts a `*o` column spec splits it
/// into (`str(outcome[i])`, as Python splits tuples, and strs into their characters).
///
/// Implemented for the integer types, `bool`, `String` and `&str`, [`Ratio`] (Python's `Fraction`), tuples,
/// arrays and `Vec`s (Python's tuples), and for `Value` with Python's own `str` (which may raise: a host object's
/// `__str__`, an int beyond CPython's 4300 digits).
///
/// ```
/// use icepool_core::FormatOutcome;
/// assert_eq!(true.fmt_outcome().unwrap(), "True");
/// assert_eq!((1, "a").fmt_outcome().unwrap(), "(1, 'a')");
/// assert_eq!([7].fmt_outcome().unwrap(), "(7,)");
/// ```
pub trait FormatOutcome {
    /// `str(outcome)`.
    ///
    /// ```
    /// use icepool_core::{FormatOutcome, Ratio};
    /// // Python: `str('a') == 'a'`, `str(Fraction(1, 2)) == '1/2'`, `str(Fraction(4, 1)) == '4'`.
    /// assert_eq!("a".fmt_outcome().unwrap(), "a");
    /// assert_eq!(Ratio::from_parts(1.into(), 2u8.into()).fmt_outcome().unwrap(), "1/2");
    /// assert_eq!(Ratio::from(4).fmt_outcome().unwrap(), "4");
    /// ```
    fn fmt_outcome(&self) -> Result<String>;

    /// `repr(outcome)`: how the outcome prints inside a tuple. The same as `str` unless overridden (strs are
    /// quoted, a `Fraction` is `Fraction(n, d)`).
    ///
    /// ```
    /// use icepool_core::{FormatOutcome, Ratio};
    /// // Python: `repr('a') == "'a'"`, `repr(Fraction(1, 2)) == 'Fraction(1, 2)'`.
    /// assert_eq!("a".repr_outcome().unwrap(), "'a'");
    /// assert_eq!(Ratio::from_parts(1.into(), 2u8.into()).repr_outcome().unwrap(), "Fraction(1, 2)");
    /// assert_eq!(5.repr_outcome().unwrap(), "5"); // the same as `str`
    /// ```
    fn repr_outcome(&self) -> Result<String> {
        self.fmt_outcome()
    }

    /// What `len(outcome)` says for `common_outcome_length`: no length (the default), a length, or a mapping.
    ///
    /// ```
    /// use icepool_core::host::LenKind;
    /// use icepool_core::FormatOutcome;
    /// assert_eq!((1, 2, 3).outcome_len().unwrap(), LenKind::Len(3));
    /// assert_eq!("ab".outcome_len().unwrap(), LenKind::Len(2));
    /// assert_eq!(7.outcome_len().unwrap(), LenKind::Unsized);
    /// ```
    fn outcome_len(&self) -> Result<LenKind> {
        Ok(LenKind::Unsized)
    }

    /// `str(outcome[i])`, the `i`th `*o` column. Only asked for an `i` below some outcome's length; the default
    /// is Python's `TypeError` for an outcome without items.
    ///
    /// ```
    /// use icepool_core::FormatOutcome;
    /// // The `*o` columns of a table: `str(outcome[i])`.
    /// assert_eq!((1, "a").fmt_part(1).unwrap(), "a");
    /// assert_eq!("xy".fmt_part(0).unwrap(), "x");
    /// // Python: `7[0]` raises TypeError.
    /// assert_eq!(7.fmt_part(0).unwrap_err().to_string(), "object is not subscriptable");
    /// ```
    fn fmt_part(&self, i: usize) -> Result<String> {
        let _ = i;
        Err(Error::WrongType("object is not subscriptable".into()))
    }
}

macro_rules! integer_outcome {
    ($($t:ty),*) => {$(
        /// In decimal, as Python's `int`.
        impl FormatOutcome for $t {
            fn fmt_outcome(&self) -> Result<String> {
                Ok(self.to_string())
            }
        }
    )*};
}

integer_outcome!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

/// `True` / `False`.
impl FormatOutcome for bool {
    fn fmt_outcome(&self) -> Result<String> {
        Ok(if *self { "True" } else { "False" }.to_string())
    }
}

/// A `Fraction`: `str` is `n/d` (`n` when whole), `repr` is `Fraction(n, d)`.
impl FormatOutcome for Ratio {
    fn fmt_outcome(&self) -> Result<String> {
        let n = int_str(self.numerator())?;
        if self.denominator().is_one() {
            Ok(n)
        } else {
            Ok(format!(
                "{n}/{}",
                int_str(&IBig::from(self.denominator().clone()))?
            ))
        }
    }

    fn repr_outcome(&self) -> Result<String> {
        Ok(format!(
            "Fraction({}, {})",
            int_str(self.numerator())?,
            int_str(&IBig::from(self.denominator().clone()))?
        ))
    }
}

/// A Python `str`: itself, quoted inside a tuple, and split into its characters by `*o`.
impl FormatOutcome for str {
    fn fmt_outcome(&self) -> Result<String> {
        Ok(self.to_string())
    }

    fn repr_outcome(&self) -> Result<String> {
        str_repr(self)
    }

    fn outcome_len(&self) -> Result<LenKind> {
        Ok(LenKind::Len(self.chars().count()))
    }

    fn fmt_part(&self, i: usize) -> Result<String> {
        Ok(self.chars().nth(i).expect("below the length").to_string())
    }
}

/// As the `str` it holds.
impl FormatOutcome for String {
    fn fmt_outcome(&self) -> Result<String> {
        self.as_str().fmt_outcome()
    }

    fn repr_outcome(&self) -> Result<String> {
        self.as_str().repr_outcome()
    }

    fn outcome_len(&self) -> Result<LenKind> {
        self.as_str().outcome_len()
    }

    fn fmt_part(&self, i: usize) -> Result<String> {
        self.as_str().fmt_part(i)
    }
}

/// As what it points to (`Die<&str>`).
impl<T: FormatOutcome + ?Sized> FormatOutcome for &T {
    fn fmt_outcome(&self) -> Result<String> {
        (**self).fmt_outcome()
    }

    fn repr_outcome(&self) -> Result<String> {
        (**self).repr_outcome()
    }

    fn outcome_len(&self) -> Result<LenKind> {
        (**self).outcome_len()
    }

    fn fmt_part(&self, i: usize) -> Result<String> {
        (**self).fmt_part(i)
    }
}

/// `str(tuple)`: the items' reprs in parentheses, with a trailing comma for a single item.
fn tuple_str(items: &[&dyn FormatOutcome]) -> Result<String> {
    let items = items
        .iter()
        .map(|x| x.repr_outcome())
        .collect::<Result<Vec<_>>>()?;
    Ok(if items.len() == 1 {
        format!("({},)", items[0])
    } else {
        format!("({})", items.join(", "))
    })
}

/// `str(tuple(items))`.
fn sequence_str<T: FormatOutcome>(items: &[T]) -> Result<String> {
    tuple_str(
        &items
            .iter()
            .map(|x| x as &dyn FormatOutcome)
            .collect::<Vec<_>>(),
    )
}

/// Python's tuple: `(1, 2)`, one `*o` column per item.
impl<T: FormatOutcome, const N: usize> FormatOutcome for [T; N] {
    fn fmt_outcome(&self) -> Result<String> {
        sequence_str(self)
    }

    fn outcome_len(&self) -> Result<LenKind> {
        Ok(LenKind::Len(N))
    }

    fn fmt_part(&self, i: usize) -> Result<String> {
        self[i].fmt_outcome()
    }
}

/// Python's tuple, as `IntoValue` makes a `Vec`: one `*o` column per item when all have the same length.
impl<T: FormatOutcome> FormatOutcome for Vec<T> {
    fn fmt_outcome(&self) -> Result<String> {
        sequence_str(self)
    }

    fn outcome_len(&self) -> Result<LenKind> {
        Ok(LenKind::Len(self.len()))
    }

    fn fmt_part(&self, i: usize) -> Result<String> {
        self[i].fmt_outcome()
    }
}

macro_rules! tuple_outcome {
    ($(($len:literal: $($t:ident $i:tt),+)),*) => {$(
        /// Python's tuple: `(1, 'a')`, one `*o` column per item.
        impl<$($t: FormatOutcome),+> FormatOutcome for ($($t,)+) {
            fn fmt_outcome(&self) -> Result<String> {
                tuple_str(&[$(&self.$i as &dyn FormatOutcome),+])
            }

            fn outcome_len(&self) -> Result<LenKind> {
                Ok(LenKind::Len($len))
            }

            fn fmt_part(&self, i: usize) -> Result<String> {
                match i {
                    $($i => self.$i.fmt_outcome(),)+
                    _ => unreachable!("only asked below the length"),
                }
            }
        }
    )*};
}

tuple_outcome!(
    (1: A 0),
    (2: A 0, B 1),
    (3: A 0, B 1, C 2),
    (4: A 0, B 1, C 2, D 3),
    (5: A 0, B 1, C 2, D 3, E 4),
    (6: A 0, B 1, C 2, D 3, E 4, F 5)
);

/// `float(x)` of a `Fraction`: correctly rounded.
fn to_float(x: &Ratio) -> Result<f64> {
    ratio_to_f64(x.numerator(), &x.denominator().clone().into())
}

/// One column of a col_spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// `o`: the outcomes.
    Outcome,
    /// `*o`: the outcomes, unpacked if applicable.
    OutcomeStar,
    /// `q==`, `q<=`, `q>=`: quantities.
    Quantity(Comparison),
    /// `p`, `%` or `i` (probabilities as 0-1, 0%-100% or "1 in N") with `==`, `<=` or `>=`.
    Probability(char, Comparison),
}

impl Token {
    /// The token's length in the spec (for `split_format_spec`'s check).
    fn len(self) -> usize {
        match self {
            Token::Outcome => 1,
            Token::OutcomeStar => 2,
            _ => 3,
        }
    }
}

/// Splits the col_spec into its components: `re.findall` of the token pattern, which must cover the whole spec.
pub fn split_format_spec(col_spec: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = col_spec.chars().collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let comparison = |j: usize| match chars.get(j..j + 2) {
            Some(['=', '=']) => Some(Comparison::Eq),
            Some(['<', '=']) => Some(Comparison::Le),
            Some(['>', '=']) => Some(Comparison::Ge),
            _ => None,
        };
        let token = match chars[i] {
            '*' if chars.get(i + 1) == Some(&'o') => Some(Token::OutcomeStar),
            'o' => Some(Token::Outcome),
            'q' => comparison(i + 1).map(Token::Quantity),
            c @ ('%' | 'i' | 'p') => comparison(i + 1).map(|cmp| Token::Probability(c, cmp)),
            _ => None,
        };
        // findall skips what does not match; the total length then falls short.
        match token {
            Some(t) => {
                result.push(t);
                i += t.len();
            }
            None => i += 1,
        }
    }
    if result.iter().map(|t| t.len()).sum::<usize>() != chars.len() {
        return Err(Error::InvalidArgument(format!(
            "Invalid col_spec '{col_spec}'"
        )));
    }
    Ok(result)
}

/// `Population.common_outcome_length`: the common length of all outcomes; `None` if outcomes have no lengths or
/// different lengths (or one is a `Mapping`).
///
/// ```
/// use icepool_core::value::common_outcome_length;
/// use icepool_core::{d, Die};
/// assert_eq!(common_outcome_length(&d(6)).unwrap(), None);
/// let pairs = Die::cartesian_product(&[d(2), d(3)]);
/// assert_eq!(common_outcome_length(&pairs).unwrap(), Some(2));
/// ```
pub fn common_outcome_length<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
) -> Result<Option<usize>> {
    let mut result = None;
    for outcome in population.outcomes() {
        match outcome.outcome_len()? {
            LenKind::Mapping => return Ok(None),
            LenKind::Len(n) => match result {
                None => result = Some(n),
                Some(r) if r != n => return Ok(None),
                _ => {}
            },
            LenKind::Unsized => {}
        }
    }
    Ok(result)
}

fn comparator(c: Comparison) -> &'static str {
    match c {
        Comparison::Eq => "==",
        Comparison::Le => "<=",
        Comparison::Ge => ">=",
        _ => unreachable!("split_format_spec only makes ==, <=, >="),
    }
}

/// Generates a list of strings for the header.
fn make_headers<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    format_tokens: &[Token],
) -> Result<Vec<String>> {
    let mut result = Vec::new();
    for &token in format_tokens {
        match token {
            Token::Outcome => result.push("Outcome".to_string()),
            Token::OutcomeStar => match common_outcome_length(population)? {
                None => result.push("Outcome".to_string()),
                Some(r) => result.extend((0..r).map(|i| format!("Outcome[{i}]"))),
            },
            Token::Quantity(c) | Token::Probability(_, c) => {
                let mut heading = match token {
                    Token::Quantity(_) => "Quantity",
                    _ => "Probability",
                }
                .to_string();
                if c != Comparison::Eq {
                    heading += " ";
                    heading += comparator(c);
                }
                result.push(heading);
            }
        }
    }
    Ok(result)
}

fn gather_cols<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    format_tokens: &[Token],
) -> Result<Vec<Vec<String>>> {
    let outcomes_str = || {
        population
            .outcomes()
            .map(O::fmt_outcome)
            .collect::<Result<Vec<_>>>()
    };
    let mut result = Vec::new();
    for &token in format_tokens {
        match token {
            Token::Outcome => result.push(outcomes_str()?),
            Token::OutcomeStar => match common_outcome_length(population)? {
                None => result.push(outcomes_str()?),
                Some(r) => {
                    for i in 0..r {
                        result.push(
                            population
                                .outcomes()
                                .map(|x| x.fmt_part(i))
                                .collect::<Result<_>>()?,
                        );
                    }
                }
            },
            Token::Quantity(c) => result.push(
                population
                    .quantities_by(c)
                    .into_iter()
                    .map(|q| int_str(&q.into()))
                    .collect::<Result<_>>()?,
            ),
            Token::Probability(kind, c) => {
                if population.denominator().is_zero() {
                    result.push(population.outcomes().map(|_| "n/a".to_string()).collect());
                    continue;
                }
                // `Fraction(quantity, denominator)`, as `probabilities` makes them.
                let denominator = population.denominator();
                let col = population
                    .quantities_by(c)
                    .into_iter()
                    .map(|q| Ratio::from_parts(q.into(), denominator.clone()));
                let cells = col.map(|x| match kind {
                    'p' => Ok(format_float(to_float(&x)?, FloatFormat::Fixed, 6)),
                    '%' => Ok(format_float(to_float(&x)?, FloatFormat::Percent, 6)),
                    // Python's arithmetic on the `Fraction`, as `format_probability_inverse` does on any number.
                    _ => format_probability_inverse(&Value::Fraction(x), &Value::from(20)),
                });
                result.push(cells.collect::<Result<_>>()?);
            }
        }
    }
    Ok(result)
}

/// The rows: one per outcome, even if there are no columns.
fn make_rows<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    format_tokens: &[Token],
) -> Result<Vec<Vec<String>>> {
    let cols = gather_cols(population, format_tokens)?;
    Ok((0..population.len())
        .map(|i| cols.iter().map(|c| c[i].clone()).collect())
        .collect())
}

/// Python's `len(str)`: code points.
fn width(s: &str) -> usize {
    s.chars().count()
}

fn compute_col_widths(headers: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut result: Vec<usize> = headers.iter().map(|s| width(s)).collect();
    for row in rows {
        result = result
            .iter()
            .zip(row)
            .map(|(&x, s)| x.max(width(s)))
            .collect();
    }
    result
}

/// `re.match(r'-?\d+(\.\d*)?%?$', cell)`. `$` also matches before a final newline. Python's `\d` is any Unicode
/// decimal digit; the core cannot tell those apart from other numeric characters, so a non-ASCII numeric
/// character is unsupported.
fn is_numeric_cell(cell: &str) -> Result<bool> {
    let chars: Vec<char> = cell.chars().collect();
    if chars.iter().any(|c| !c.is_ascii() && c.is_numeric()) {
        return Err(Error::Unported("a non-ASCII digit in a table cell".into()));
    }
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while chars.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    if chars.first() == Some(&'-') {
        i += 1;
    }
    if digits(&mut i) == 0 {
        return Ok(false);
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        digits(&mut i);
    }
    if chars.get(i) == Some(&'%') {
        i += 1;
    }
    Ok(i == chars.len() || (i + 1 == chars.len() && chars[i] == '\n'))
}

/// For each column, whether it is aligned right: iff all its values are numeric.
fn compute_alignments(rows: &[Vec<String>]) -> Result<Vec<bool>> {
    let Some(first) = rows.first() else {
        return Ok(vec![]);
    };
    let mut result = vec![true; first.len()];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if !is_numeric_cell(cell)? {
                result[i] = false;
            }
        }
    }
    Ok(result)
}

/// `f'{s:{alignment}{width}}'`.
fn pad(s: &str, right: bool, col_width: usize) -> String {
    let fill = " ".repeat(col_width.saturating_sub(width(s)));
    if right {
        fill + s
    } else {
        s.to_string() + &fill
    }
}

fn empty<O: OutcomeOrder + FormatOutcome>(population: &Die<O>, type_name: &str) -> Option<String> {
    population
        .is_empty()
        .then(|| format!("Empty {type_name}\n"))
}

fn caption<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
) -> Result<String> {
    Ok(format!(
        "{type_name} with denominator {}",
        int_str(&population.denominator().into())?
    ))
}

/// The table's headers and rows, as `csv` writes them.
pub fn table<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    col_spec: &str,
) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let format_tokens = split_format_spec(col_spec)?;
    let headers = make_headers(population, &format_tokens)?;
    let rows = make_rows(population, &format_tokens)?;
    Ok((headers, rows))
}

/// Formats the population as a Markdown table. `type_name` is `type(population).__name__`.
pub fn markdown<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
    col_spec: &str,
) -> Result<String> {
    if let Some(e) = empty(population, type_name) {
        return Ok(e);
    }
    let (headers, rows) = table(population, col_spec)?;
    let col_widths = compute_col_widths(&headers, &rows);
    let alignments = compute_alignments(&rows)?;

    let mut result = caption(population, type_name)? + "\n\n";
    result += "|";
    for ((header, &right), &w) in headers.iter().zip(&alignments).zip(&col_widths) {
        result += &format!(" {} |", pad(header, right, w));
    }
    result += "\n";

    result += "|";
    for (&right, &w) in alignments.iter().zip(&col_widths) {
        if right {
            result += &format!("-{}:|", "-".repeat(w));
        } else {
            result += &format!(":{}-|", "-".repeat(w));
        }
    }
    result += "\n";

    for row in &rows {
        result += "|";
        for ((s, &right), &w) in row.iter().zip(&alignments).zip(&col_widths) {
            result += &format!(" {} |", pad(s, right, w));
        }
        result += "\n";
    }

    result += "\n";
    Ok(result)
}

/// One row as Python's `csv.writer` with the default `'excel'` dialect writes it (`QUOTE_MINIMAL`, `,`, `"`
/// doubled, `\r\n`): a field is quoted if it holds the delimiter, the quote character, `\n` or `\r`; a row made of
/// a single empty field is written as `""` (an empty row as nothing).
fn csv_row(fields: &[String]) -> String {
    if let [only] = fields {
        if only.is_empty() {
            return "\"\"\r\n".to_string();
        }
    }
    let quoted = fields.iter().map(|f| {
        if f.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", f.replace('"', "\"\""))
        } else {
            f.clone()
        }
    });
    quoted.collect::<Vec<_>>().join(",") + "\r\n"
}

/// Formats the population as a comma-separated-values table (the `'excel'` dialect; the facade writes other
/// dialects and format parameters with its own `csv` module, from `table`).
pub fn csv<O: OutcomeOrder + FormatOutcome>(population: &Die<O>, col_spec: &str) -> Result<String> {
    let (headers, rows) = table(population, col_spec)?;
    let mut out = csv_row(&headers);
    for row in &rows {
        out += &csv_row(row);
    }
    Ok(out)
}

/// Formats the population as a BBCode table.
pub fn bbcode<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
    col_spec: &str,
) -> Result<String> {
    if let Some(e) = empty(population, type_name) {
        return Ok(e);
    }
    let (headers, rows) = table(population, col_spec)?;
    let alignments = compute_alignments(&rows)?;

    let mut result = caption(population, type_name)? + "\n\n";
    result += "[table]\n";
    result += "[tr]";
    for (header, &right) in headers.iter().zip(&alignments) {
        if right {
            result += &format!("[th][right]{header}[/right][/th]");
        } else {
            result += &format!("[th]{header}[/th]");
        }
    }
    result += "[/tr]\n";

    for row in &rows {
        result += "[tr]";
        for (s, &right) in row.iter().zip(&alignments) {
            if right {
                result += &format!("[td][right]{s}[/right][/td]");
            } else {
                result += &format!("[td]{s}[/td]");
            }
        }
        result += "[/tr]\n";
    }

    result += "[/table]\n";
    Ok(result)
}

/// `html.escape(s)` (with `quote=True`).
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Formats the population as a HTML table.
pub fn html<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
    col_spec: &str,
) -> Result<String> {
    if let Some(e) = empty(population, type_name) {
        return Ok(e);
    }
    let (headers, rows) = table(population, col_spec)?;
    let alignments = compute_alignments(&rows)?;

    let mut result = "<table>\n".to_string();
    result += &format!("<caption>{}</caption>\n", caption(population, type_name)?);
    result += "<tr>";
    for (header, &right) in headers.iter().zip(&alignments) {
        let header = html_escape(header);
        if right {
            result += &format!("<th style=\"text-align:right;\">{header}</th>");
        } else {
            result += &format!("<th>{header}</th>");
        }
    }
    result += "</tr>\n";

    for row in &rows {
        result += "<tr>";
        for (s, &right) in row.iter().zip(&alignments) {
            let s = html_escape(s);
            if right {
                result += &format!("<td style=\"text-align:right;\">{s}</td>");
            } else {
                result += &format!("<td>{s}</td>");
            }
        }
        result += "</tr>\n";
    }

    result += "</table>\n";
    Ok(result)
}

impl<O: OutcomeOrder + FormatOutcome> Die<O> {
    /// Python's `die.format(format_spec)` (`f'{die:format_spec}'`): the output format (`md`, `bbcode`, `csv`,
    /// `html`), then optionally `:` and the columns. An empty spec is `str(die)`, a markdown table. `ValueError`
    /// for a spec Python refuses.
    ///
    /// ```
    /// use icepool_core::d;
    /// assert_eq!(
    ///     d(2).format("csv:oq==").unwrap(),
    ///     "Outcome,Quantity\r\n1,1\r\n2,1\r\n"
    /// );
    /// assert!(d(2).format("xml").is_err());
    /// ```
    pub fn format(&self, format_spec: &str) -> Result<String> {
        format_text(self, "Die", format_spec)
    }

    /// Python's `repr(die)`: `Die({outcome: quantity, ...})`, each outcome as Python's `repr`.
    ///
    /// ```
    /// use icepool_core::{d, Die};
    /// assert_eq!(d(3).repr().unwrap(), "Die({1: 1, 2: 1, 3: 1})");
    /// assert_eq!(Die::from_iter(["a".to_string()]).repr().unwrap(), "Die({'a': 1})");
    /// ```
    pub fn repr(&self) -> Result<String> {
        population_repr("Die", self.items())
    }
}

impl<O: OutcomeOrder + FormatOutcome> Deck<O> {
    /// Python's `deck.format(format_spec)`: as [`Die::format`], with a `Deck` header.
    ///
    /// ```
    /// use icepool_core::Deck;
    /// let deck = Deck::from_iter([1, 1, 2]);
    /// assert_eq!(deck.format("csv:oq==").unwrap(), "Outcome,Quantity\r\n1,2\r\n2,1\r\n");
    /// ```
    pub fn format(&self, format_spec: &str) -> Result<String> {
        format_text(self.population(), "Deck", format_spec)
    }

    /// Python's `repr(deck)`: `Deck({outcome: quantity, ...})`, each outcome as Python's `repr`.
    ///
    /// ```
    /// use icepool_core::Deck;
    /// assert_eq!(Deck::from_iter([1, 1, 2]).repr().unwrap(), "Deck({1: 2, 2: 1})");
    /// ```
    pub fn repr(&self) -> Result<String> {
        population_repr("Deck", self.items())
    }
}

/// `Population.format(format_spec)` without CSV writer arguments: always text.
fn format_text<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
    format_spec: &str,
) -> Result<String> {
    match format(population, type_name, format_spec, false)? {
        Formatted::Text(text) => Ok(text),
        Formatted::CsvTable(..) => unreachable!("only asked for with csv_table"),
    }
}

/// die.py / deck.py `__repr__`: `type(self).__qualname__ + '({' + items_string + '})'`, where `items_string` joins
/// `f'{repr(outcome)}: {quantity}'`.
fn population_repr<O: FormatOutcome>(type_name: &str, items: &[(O, Quantity)]) -> Result<String> {
    let items = items
        .iter()
        .map(|(o, q)| Ok(format!("{}: {}", o.repr_outcome()?, int_str(&IBig::from(q.clone()))?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(format!("{type_name}({{{}}})", items.join(", ")))
}

/// Python's `str(pool)` (pool.py `Pool.__str__`): a header with the number of dice and the keep tuple, then one
/// line `  repr(die) : count,` per distinct die.
fn pool_str<O: OutcomeOrder + FormatOutcome>(pool: &Pool<O>) -> Result<String> {
    let mut text = format!(
        "Pool of {} dice with keep_tuple={}\n",
        pool.raw_size(),
        keep_tuple_repr(pool.keep_tuple())
    );
    for (die, count) in pool.dice() {
        text += &format!("  {} : {count},\n", die.repr()?);
    }
    Ok(text)
}

/// Python's `repr` of a tuple of ints: `(1, 1)`, and `(1,)` for one element.
fn keep_tuple_repr(keep_tuple: &[Int]) -> String {
    match keep_tuple {
        [single] => format!("({single},)"),
        _ => {
            let parts: Vec<String> = keep_tuple.iter().map(Int::to_string).collect();
            format!("({})", parts.join(", "))
        }
    }
}

/// The text of a `Display` impl, which can only say that it failed: where the port cannot print as Python (a
/// non-ASCII str inside a tuple, whose `repr` needs Unicode's tables, or an integer of more than 4300 digits).
fn write_or_fail(f: &mut std::fmt::Formatter<'_>, text: Result<String>) -> std::fmt::Result {
    match text {
        Ok(text) => f.write_str(&text),
        Err(_) => Err(std::fmt::Error),
    }
}

/// Python's `str(pool)`: a header with the number of dice and the keep tuple, then `repr(die) : count` for each
/// distinct die. Fails (`fmt::Error`) only where [`Die`]'s `Display` does.
///
/// ```
/// use icepool_core::d;
/// assert_eq!(
///     d(4).pool(3).highest(2).to_string(),
///     "Pool of 3 dice with keep_tuple=(0, 1, 1)\n  Die({1: 1, 2: 1, 3: 1, 4: 1}) : 3,\n"
/// );
/// ```
impl<O: Outcome + FormatOutcome> std::fmt::Display for Pool<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_or_fail(f, pool_str(self))
    }
}

/// Python's `str(deck)`: a markdown table, as a die's, with a `Deck` header. Fails (`fmt::Error`) only where
/// [`Die`]'s `Display` does.
///
/// ```
/// use icepool_core::Deck;
/// let text = Deck::from_iter([1, 1, 2]).to_string();
/// assert!(text.starts_with("Deck with denominator 3\n"));
/// assert!(text.contains("|       1 |        2 |  66.666667% |"));
/// ```
impl<O: Outcome + FormatOutcome> std::fmt::Display for Deck<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write_or_fail(f, self.format(""))
    }
}

/// Python's `str(deal)` (deal.py `Deal.__str__`): `Deal of hand_size=n from deck:` and the deck's table.
///
/// ```
/// use icepool_core::Deck;
/// let text = Deck::from_iter([1, 2, 3]).deal(2).to_string();
/// assert!(text.starts_with("Deal of hand_size=2 from deck:\nDeck with denominator 3\n"));
/// ```
impl<O: Outcome + FormatOutcome> std::fmt::Display for Deal<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Deal of hand_size={} from deck:\n{}", self.hand_size(), self.deck())
    }
}

/// What `Population.format` produces ([`format_population`](crate::value::format_population)).
///
/// ```
/// use icepool_core::value::{format_population, Formatted};
/// use icepool_core::d;
/// // With `csv_table`, the CSV comes back as its header and rows.
/// let Formatted::CsvTable(header, rows) = format_population(&d(2), "Die", "csv", true).unwrap() else { panic!() };
/// assert_eq!(header, ["Outcome", "Quantity", "Probability"]);
/// assert_eq!(rows[0], ["1", "1", "50.000000%"]);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Formatted {
    /// The formatted text.
    Text(String),
    /// A CSV table for the caller's own `csv` writer (`format` was given writer arguments).
    CsvTable(Vec<String>, Vec<Vec<String>>),
}

/// `Population.format(format_spec)`: the output format (`md`, `bbcode`, `csv`, `html`), then optionally `:` and the
/// columns (`|` separators are dropped). The default columns are `*oq==%==`, without the quantities if any of them
/// is 10**30 or greater. With `csv_table`, CSV output is returned as its table (Python passes `**kwargs` to
/// `csv.writer`).
///
/// For language bindings: `type_name` is the population's class name (`Die`, `Deck`) for the table header.
///
/// ```
/// use icepool_core::value::{format_population, Formatted};
/// use icepool_core::d;
/// let Formatted::Text(csv) = format_population(&d(2), "Die", "csv", false).unwrap() else { panic!() };
/// assert_eq!(csv, "Outcome,Quantity,Probability\r\n1,1,50.000000%\r\n2,1,50.000000%\r\n");
/// assert!(format_population(&d(2), "Die", "tex", false).is_err());
/// ```
pub fn format<O: OutcomeOrder + FormatOutcome>(
    population: &Die<O>,
    type_name: &str,
    format_spec: &str,
    csv_table: bool,
) -> Result<Formatted> {
    let default_column_spec = if !population.is_empty()
        && population.modal_quantity()? < IBig::from(10).pow(30).try_into().expect("positive")
    {
        "*oq==%=="
    } else {
        "*o%=="
    };
    let format_spec = if format_spec.is_empty() {
        format!("md:{default_column_spec}")
    } else {
        format_spec.to_string()
    };
    let format_spec = format_spec.replace('|', "");
    let parts: Vec<&str> = format_spec.split(':').collect();
    let (output_format, col_spec) = match parts[..] {
        [output_format] => (output_format, default_column_spec),
        [output_format, col_spec] => (output_format, col_spec),
        _ => {
            return Err(Error::InvalidArgument(
                "format_spec has too many colons.".into(),
            ))
        }
    };
    Ok(Formatted::Text(match output_format {
        "md" => markdown(population, type_name, col_spec)?,
        "bbcode" => bbcode(population, type_name, col_spec)?,
        "csv" if csv_table => {
            let (headers, rows) = table(population, col_spec)?;
            return Ok(Formatted::CsvTable(headers, rows));
        }
        "csv" => csv(population, col_spec)?,
        "html" => html(population, type_name, col_spec)?,
        _ => {
            return Err(Error::InvalidArgument(format!(
                "Unsupported output format '{output_format}'"
            )))
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_tokens() {
        assert_eq!(
            split_format_spec("*oq==%<=i>=").unwrap(),
            vec![
                Token::OutcomeStar,
                Token::Quantity(Comparison::Eq),
                Token::Probability('%', Comparison::Le),
                Token::Probability('i', Comparison::Ge)
            ]
        );
        assert!(split_format_spec("oq=").is_err());
        assert!(split_format_spec("x").is_err());
        assert_eq!(split_format_spec("").unwrap(), vec![]);
    }

    /// `str(Die(...))` of the same outcomes in Python, as the oracle prints it.
    #[test]
    fn typed_dice_print_as_python() {
        use crate::{d, Quantity};
        let table = |rows: &str| -> String {
            let (caption, body) = rows.split_once('|').unwrap();
            format!("Die with denominator {caption}\n\n|{body}\n")
        };
        // `Die([True, False, False])`
        assert_eq!(
            Die::from_iter([true, false, false]).to_string(),
            table(
                "3| Outcome | Quantity | Probability |\n|:--------|---------:|------------:|\n\
                 | False   |        2 |  66.666667% |\n| True    |        1 |  33.333333% |\n"
            )
        );
        // `Die(['ab', 'cd'])`: strs of one length split into their characters; `Die(['ab', 'cd', 'x'])` do not.
        assert_eq!(
            Die::from_iter(["ab", "cd"]).to_string(),
            table(
                "2| Outcome[0] | Outcome[1] | Quantity | Probability |\n\
                 |:-----------|:-----------|---------:|------------:|\n\
                 | a          | b          |        1 |  50.000000% |\n\
                 | c          | d          |        1 |  50.000000% |\n"
            )
        );
        assert_eq!(
            Die::from_iter(["ab".to_string(), "cd".into(), "x".into()]).to_string(),
            table(
                "3| Outcome | Quantity | Probability |\n|:--------|---------:|------------:|\n\
                 | ab      |        1 |  33.333333% |\n| cd      |        1 |  33.333333% |\n\
                 | x       |        1 |  33.333333% |\n"
            )
        );
        // `Die([(1, 'a'), (2, 'b')])`, `Die([(1, 2), (3, 4), (3, 4)])`
        assert_eq!(
            Die::from_iter([(1, "a"), (2, "b")]).to_string(),
            table(
                "2| Outcome[0] | Outcome[1] | Quantity | Probability |\n\
                 |-----------:|:-----------|---------:|------------:|\n\
                 |          1 | a          |        1 |  50.000000% |\n\
                 |          2 | b          |        1 |  50.000000% |\n"
            )
        );
        assert_eq!(
            Die::from_iter([[1, 2], [3, 4], [3, 4]]).to_string(),
            table(
                "3| Outcome[0] | Outcome[1] | Quantity | Probability |\n\
                 |-----------:|-----------:|---------:|------------:|\n\
                 |          1 |          2 |        1 |  33.333333% |\n\
                 |          3 |          4 |        2 |  66.666667% |\n"
            )
        );
        // `Die([(1,), (1, 2)])`: different lengths make one column; `Die([()])` none at all.
        assert_eq!(
            Die::from_iter([vec![1], vec![1, 2]]).to_string(),
            table(
                "2| Outcome | Quantity | Probability |\n|:--------|---------:|------------:|\n\
                 | (1,)    |        1 |  50.000000% |\n| (1, 2)  |        1 |  50.000000% |\n"
            )
        );
        assert_eq!(
            Die::from_iter([Vec::<i64>::new()]).to_string(),
            table("1| Quantity | Probability |\n|---------:|------------:|\n|        1 | 100.000000% |\n")
        );
        // `Die([Fraction(1, 2), Fraction(3, 1)])`, `Die([((1, 2), 'x')])`
        assert_eq!(
            Die::from_iter([Ratio::from_parts(1.into(), 2u8.into()), Ratio::from(3)]).to_string(),
            table(
                "2| Outcome | Quantity | Probability |\n|:--------|---------:|------------:|\n\
                 | 1/2     |        1 |  50.000000% |\n| 3       |        1 |  50.000000% |\n"
            )
        );
        assert_eq!(
            Die::single(((1, 2), "x")).to_string(),
            table(
                "1| Outcome[0] | Outcome[1] | Quantity | Probability |\n\
                 |:-----------|:-----------|---------:|------------:|\n\
                 | (1, 2)     | x          |        1 | 100.000000% |\n"
            )
        );
        // `Die({1: 10**30, 2: 1})`: no quantity column once one reaches 10**30; `Die([])`.
        let huge = Quantity::from(10u8).pow(30);
        assert_eq!(
            Die::from_weights([(1, huge), (2, Quantity::ONE)]).to_string(),
            table(
                "1000000000000000000000000000001| Outcome | Probability |\n|--------:|------------:|\n\
                 |       1 | 100.000000% |\n|       2 |   0.000000% |\n"
            )
        );
        assert_eq!(Die::<i64>::from_iter([]).to_string(), "Empty Die\n");
        // `f'{d6:csv:oq<=p>=i==}'`, `f'{d(3):bbcode:o%<=}'`
        assert_eq!(
            d(6).format("csv:oq<=p>=i==").unwrap(),
            "Outcome,Quantity <=,Probability >=,Probability\r\n1,1,1.000000,1 in 6.0\r\n\
             2,2,0.833333,1 in 6.0\r\n3,3,0.666667,1 in 6.0\r\n4,4,0.500000,1 in 6.0\r\n\
             5,5,0.333333,1 in 6.0\r\n6,6,0.166667,1 in 6.0\r\n"
        );
        assert_eq!(
            d(3).format("bbcode:o%<=").unwrap(),
            "Die with denominator 3\n\n[table]\n\
             [tr][th][right]Outcome[/right][/th][th][right]Probability <=[/right][/th][/tr]\n\
             [tr][td][right]1[/right][/td][td][right]33.333333%[/right][/td][/tr]\n\
             [tr][td][right]2[/right][/td][td][right]66.666667%[/right][/td][/tr]\n\
             [tr][td][right]3[/right][/td][td][right]100.000000%[/right][/td][/tr]\n[/table]\n"
        );
    }

    /// A typed die prints as the same die of `Value`s.
    #[test]
    fn typed_and_value_dice_print_alike() {
        use crate::value::evaluators::IntoValue;
        let typed = Die::from_iter([(1i64, true), (2, false), (2, false), (-3, true)]);
        let as_values = Die::<Value>::from_dict(
            typed
                .items()
                .iter()
                .map(|(o, q)| ((*o).into_value(), q.clone()))
                .collect(),
        )
        .unwrap();
        for spec in ["", "md:*oq>=i<=", "html:o", "csv:%=="] {
            assert_eq!(
                typed.format(spec).unwrap(),
                as_values.format(spec).unwrap(),
                "{spec}"
            );
        }
    }

    #[test]
    fn numeric_cells() {
        for (cell, numeric) in [
            ("12", true),
            ("-1.5%", true),
            ("3.", true),
            ("4\n", true),
            ("4\n\n", false),
            ("-", false),
            (".5", false),
            ("1e5", false),
            ("n/a", false),
        ] {
            assert_eq!(is_numeric_cell(cell).unwrap(), numeric, "{cell:?}");
        }
    }
}
