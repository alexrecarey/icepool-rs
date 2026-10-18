//! population/format.py: a population as a Markdown, BBCode, HTML or CSV table, `Population.format`'s spec
//! parsing, and `format_probability_inverse` ("1 in N").
//!
//! Every cell is Python's `str()` of something (`value::pystr`), so outcome types print as in Python.

use dashu_int::IBig;

use crate::error::{Error, Result};
use crate::population::Comparison;
use crate::value::builtins::{float_ceil, getitem, math_log10, to_float, Index};
use crate::value::pystr::{format_float, py_str, FloatFormat};
use crate::host::LenKind;
use crate::value::{BinOp, PyStatistics};
use crate::{Die, Value};

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

/// EXPERIMENTAL: Formats the inverse of a value as "1 in N".
///
/// If N = 1 / probability is between `int_start` and 1 million times `int_start` it will be formatted as an
/// integer. Otherwise it be formatted as a float with precision at least 1 part in `int_start`.
///
/// Every step is Python's operator on the given objects (`Value::binary`), so any number type behaves as there.
pub fn format_probability_inverse(probability: &Value, int_start: &Value) -> Result<String> {
    let max_precision = float_ceil(math_log10(int_start)?)?;
    let cmp = |op, x: &Value, y: i64| -> Result<bool> { x.binary(op, &Value::from(y))?.truthy() };
    if cmp(BinOp::Le, probability, 0)? || cmp(BinOp::Gt, probability, 1)? {
        return Ok("n/a".into());
    }
    let product = probability.binary(BinOp::Mul, int_start)?;
    let inverse = || Value::Float(1.0).binary(BinOp::TrueDiv, probability);
    if cmp(BinOp::Le, &product, 1)? {
        let scaled = product.binary(BinOp::Mul, &Value::from(1_000_000))?;
        if cmp(BinOp::Le, &scaled, 1)? {
            return Ok(format!(
                "1 in {}",
                format_value(&inverse()?, FloatFormat::Exponent, &max_precision)?
            ));
        } else {
            let n = Value::from(1)
                .binary(BinOp::TrueDiv, probability)?
                .round(&Value::None)?;
            return Ok(format!("1 in {}", py_str(&n)?));
        }
    }

    let mut precision = IBig::ZERO;
    let mut precision_factor = Value::from(1);
    while cmp_values(BinOp::Gt, &product, &precision_factor)? && precision < max_precision {
        precision += IBig::ONE;
        precision_factor = precision_factor.binary(BinOp::Mul, &Value::from(10))?;
    }
    Ok(format!(
        "1 in {}",
        format_value(&inverse()?, FloatFormat::Fixed, &precision)?
    ))
}

fn cmp_values(op: BinOp, a: &Value, b: &Value) -> Result<bool> {
    a.binary(op, b)?.truthy()
}

/// `f'{x:<.{precision}{kind}}'` for the float `x` (no width, so `<` changes nothing). A negative precision makes
/// the spec unparsable (`'<.-2e'`), a `ValueError` in Python.
fn format_value(x: &Value, kind: FloatFormat, precision: &IBig) -> Result<String> {
    let Value::Float(x) = x else {
        return Err(Error::Unported(format!(
            "format() of a {} in format_probability_inverse",
            x.type_name()
        )));
    };
    if *precision < IBig::ZERO {
        return Err(Error::InvalidArgument(
            "Format specifier missing precision".into(),
        ));
    }
    match usize::try_from(precision) {
        Ok(p) if p <= 10_000 => Ok(format_float(*x, kind, p)),
        _ => Err(Error::Unported("a huge format precision".into())),
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
pub fn common_outcome_length(population: &Die<Value>) -> Result<Option<usize>> {
    let mut result = None;
    for outcome in population.outcomes() {
        let kind = match outcome {
            Value::Tuple(t) => LenKind::Len(t.len()),
            Value::Str(s) => LenKind::Len(s.chars().count()),
            Value::Host(h) => h.len_kind()?,
            Value::Vector(v) => LenKind::Len(v.data()?.len()),
            Value::Symbols(_) => LenKind::Mapping,
            _ => LenKind::Unsized,
        };
        match kind {
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
fn make_headers(population: &Die<Value>, format_tokens: &[Token]) -> Result<Vec<String>> {
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

fn gather_cols(population: &Die<Value>, format_tokens: &[Token]) -> Result<Vec<Vec<String>>> {
    let outcomes_str = || {
        population
            .outcomes()
            .map(py_str)
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
                        let index = Index::Item(Value::from(i as i64));
                        result.push(
                            population
                                .outcomes()
                                .map(|x| py_str(&getitem(x, &index)?))
                                .collect::<Result<_>>()?,
                        );
                    }
                }
            },
            Token::Quantity(c) => result.push(
                population
                    .quantities_by(c)
                    .into_iter()
                    .map(|q| py_str(&Value::Int(q.into())))
                    .collect::<Result<_>>()?,
            ),
            Token::Probability(kind, c) => {
                if population.denominator().is_zero() {
                    result.push(population.outcomes().map(|_| "n/a".to_string()).collect());
                    continue;
                }
                let col = population.probabilities(c, false)?;
                let cells = col.iter().map(|x| match kind {
                    'p' => Ok(format_float(to_float(x)?, FloatFormat::Fixed, 6)),
                    '%' => Ok(format_float(to_float(x)?, FloatFormat::Percent, 6)),
                    _ => format_probability_inverse(x, &Value::from(20)),
                });
                result.push(cells.collect::<Result<_>>()?);
            }
        }
    }
    Ok(result)
}

/// The rows: `zip(*cols)`, as long as the shortest column (`q>=` of an empty population has one entry).
fn make_rows(population: &Die<Value>, format_tokens: &[Token]) -> Result<Vec<Vec<String>>> {
    let cols = gather_cols(population, format_tokens)?;
    let n = cols.iter().map(Vec::len).min().unwrap_or(0);
    Ok((0..n)
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

/// For each column, whether it is aligned right: iff all its values are numeric. Like Python, this needs at
/// least one row (`rows[0]`).
fn compute_alignments(rows: &[Vec<String>]) -> Result<Vec<bool>> {
    let Some(first) = rows.first() else {
        return Err(Error::IndexOutOfRange("list index out of range".into()));
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

fn empty(population: &Die<Value>, type_name: &str) -> Option<String> {
    population
        .is_empty()
        .then(|| format!("Empty {type_name}\n"))
}

fn caption(population: &Die<Value>, type_name: &str) -> Result<String> {
    Ok(format!(
        "{type_name} with denominator {}",
        py_str(&Value::Int(population.denominator().into()))?
    ))
}

/// The table's headers and rows, as `csv` writes them.
pub fn table(population: &Die<Value>, col_spec: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let format_tokens = split_format_spec(col_spec)?;
    let headers = make_headers(population, &format_tokens)?;
    let rows = make_rows(population, &format_tokens)?;
    Ok((headers, rows))
}

/// Formats the population as a Markdown table. `type_name` is `type(population).__name__`.
pub fn markdown(population: &Die<Value>, type_name: &str, col_spec: &str) -> Result<String> {
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
pub fn csv(population: &Die<Value>, col_spec: &str) -> Result<String> {
    let (headers, rows) = table(population, col_spec)?;
    let mut out = csv_row(&headers);
    for row in &rows {
        out += &csv_row(row);
    }
    Ok(out)
}

/// Formats the population as a BBCode table.
pub fn bbcode(population: &Die<Value>, type_name: &str, col_spec: &str) -> Result<String> {
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
pub fn html(population: &Die<Value>, type_name: &str, col_spec: &str) -> Result<String> {
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

/// What `Population.format` produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Formatted {
    Text(String),
    /// A CSV table for the caller's own `csv` writer (`format` was given writer arguments).
    CsvTable(Vec<String>, Vec<Vec<String>>),
}

/// `Population.format(format_spec)`: the output format (`md`, `bbcode`, `csv`, `html`), then optionally `:` and the
/// columns (`|` separators are dropped). The default columns are `*oq==%==`, without the quantities if any of them
/// is 10**30 or greater. With `csv_table`, CSV output is returned as its table (Python passes `**kwargs` to
/// `csv.writer`).
pub fn format(
    population: &Die<Value>,
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

    #[test]
    fn inverse() {
        let f = |n: i64, d: u64| Value::Fraction(dashu_ratio::RBig::from_parts(n.into(), d.into()));
        let twenty = Value::from(20);
        assert_eq!(
            format_probability_inverse(&f(1, 6), &twenty).unwrap(),
            "1 in 6.0"
        );
        assert_eq!(
            format_probability_inverse(&f(1, 100), &twenty).unwrap(),
            "1 in 100"
        );
        assert_eq!(
            format_probability_inverse(&f(1, 100_000_000), &twenty).unwrap(),
            "1 in 1.00e+08"
        );
        assert_eq!(
            format_probability_inverse(&f(0, 1), &twenty).unwrap(),
            "n/a"
        );
    }
}
