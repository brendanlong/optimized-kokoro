//! English number names, written from the observed output of num2words 0.5.14 (the version misaki calls)
//! for the four conversions misaki uses: cardinal, ordinal, year and float.
//!
//! - Cardinals group by thousands up to "centillion" (10^303); 10^306 and above is an error.
//!   "and" joins a final part under 100 ("one thousand and one"), commas join the rest
//!   ("one million, two hundred thousand").
//! - Integral floats are named by repeatedly dividing with Python's float `divmod`, so above 2^53 the
//!   name follows float arithmetic rather than the exact value.
//! - Other floats: the integer part, "point", then one word per digit. The digit count comes from the
//!   float's shortest repr; the digits are the fractional part times 10^count, rounded when within 0.01
//!   of an integer and truncated otherwise.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// num2words raises `OverflowError` for these.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooLarge => f.write_str("number too large to name"),
        }
    }
}

impl std::error::Error for Error {}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// Names of 10^(3k) for k = 1..=101.
const SCALES: [&str; 101] = [
    "thousand",
    "million",
    "billion",
    "trillion",
    "quadrillion",
    "quintillion",
    "sextillion",
    "septillion",
    "octillion",
    "nonillion",
    "decillion",
    "undecillion",
    "duodecillion",
    "tredecillion",
    "quattuordecillion",
    "quindecillion",
    "sexdecillion",
    "septdecillion",
    "octodecillion",
    "novemdecillion",
    "vigintillion",
    "unvigintillion",
    "duovigintillion",
    "trevigintillion",
    "quattuorvigintillion",
    "quinvigintillion",
    "sexvigintillion",
    "septvigintillion",
    "octovigintillion",
    "novemvigintillion",
    "trigintillion",
    "untrigintillion",
    "duotrigintillion",
    "tretrigintillion",
    "quattuortrigintillion",
    "quintrigintillion",
    "sextrigintillion",
    "septtrigintillion",
    "octotrigintillion",
    "novemtrigintillion",
    "quadragintillion",
    "unquadragintillion",
    "duoquadragintillion",
    "trequadragintillion",
    "quattuorquadragintillion",
    "quinquadragintillion",
    "sexquadragintillion",
    "septquadragintillion",
    "octoquadragintillion",
    "novemquadragintillion",
    "quinquagintillion",
    "unquinquagintillion",
    "duoquinquagintillion",
    "trequinquagintillion",
    "quattuorquinquagintillion",
    "quinquinquagintillion",
    "sexquinquagintillion",
    "septquinquagintillion",
    "octoquinquagintillion",
    "novemquinquagintillion",
    "sexagintillion",
    "unsexagintillion",
    "duosexagintillion",
    "tresexagintillion",
    "quattuorsexagintillion",
    "quinsexagintillion",
    "sexsexagintillion",
    "septsexagintillion",
    "octosexagintillion",
    "novemsexagintillion",
    "septuagintillion",
    "unseptuagintillion",
    "duoseptuagintillion",
    "treseptuagintillion",
    "quattuorseptuagintillion",
    "quinseptuagintillion",
    "sexseptuagintillion",
    "septseptuagintillion",
    "octoseptuagintillion",
    "novemseptuagintillion",
    "octogintillion",
    "unoctogintillion",
    "duooctogintillion",
    "treoctogintillion",
    "quattuoroctogintillion",
    "quinoctogintillion",
    "sexoctogintillion",
    "septoctogintillion",
    "octooctogintillion",
    "novemoctogintillion",
    "nonagintillion",
    "unnonagintillion",
    "duononagintillion",
    "trenonagintillion",
    "quattuornonagintillion",
    "quinnonagintillion",
    "sexnonagintillion",
    "septnonagintillion",
    "octononagintillion",
    "novemnonagintillion",
    "centillion",
];

/// Largest number of decimal digits a nameable number has.
const MAX_DIGITS: usize = 3 * SCALES.len() + 3;

fn push_below_thousand(out: &mut String, n: u32) {
    debug_assert!(n < 1000);
    let (hundreds, rest) = (n / 100, n % 100);
    if hundreds > 0 {
        out.push_str(ONES[hundreds as usize]);
        out.push_str(" hundred");
        if rest == 0 {
            return;
        }
        out.push_str(" and ");
    }
    if rest < 20 {
        out.push_str(ONES[rest as usize]);
    } else {
        out.push_str(TENS[(rest / 10) as usize]);
        if rest % 10 > 0 {
            out.push('-');
            out.push_str(ONES[(rest % 10) as usize]);
        }
    }
}

fn digits_value(digits: &[u8]) -> u32 {
    digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0'))
}

/// `digits`: ASCII digits without leading zeros, at most `MAX_DIGITS` long.
fn push_cardinal_digits(out: &mut String, digits: &[u8]) {
    if digits.len() <= 3 {
        push_below_thousand(out, digits_value(digits));
        return;
    }
    let scale = (digits.len() - 1) / 3;
    let (high, low) = digits.split_at(digits.len() - 3 * scale);
    push_below_thousand(out, digits_value(high));
    out.push(' ');
    out.push_str(SCALES[scale - 1]);
    let low = &low[low.iter().position(|&d| d != b'0').unwrap_or(low.len())..];
    if !low.is_empty() {
        out.push_str(if low.len() <= 2 { " and " } else { ", " });
        push_cardinal_digits(out, low);
    }
}

/// `num2words(int(digits))` for a string of ASCII digits.
pub fn cardinal(digits: &str) -> Result<String, Error> {
    let digits = digits.trim_start_matches('0').as_bytes();
    if digits.len() > MAX_DIGITS {
        return Err(Error::TooLarge);
    }
    let mut out = String::new();
    push_cardinal_digits(&mut out, if digits.is_empty() { b"0" } else { digits });
    Ok(out)
}

pub fn cardinal_u64(n: u64) -> String {
    cardinal(&n.to_string()).expect("u64 is nameable")
}

/// `num2words(int(digits), to='ordinal')`
pub fn ordinal(digits: &str) -> Result<String, Error> {
    let mut out = cardinal(digits)?;
    let start = out.rfind([' ', '-']).map_or(0, |i| i + 1);
    let last = &out[start..];
    let irregular = match last {
        "one" => Some("first"),
        "two" => Some("second"),
        "three" => Some("third"),
        "five" => Some("fifth"),
        "eight" => Some("eighth"),
        "nine" => Some("ninth"),
        "twelve" => Some("twelfth"),
        _ => None,
    };
    if let Some(word) = irregular {
        out.truncate(start);
        out.push_str(word);
    } else if let Some(stem) = last.strip_suffix('y') {
        out.truncate(start + stem.len());
        out.push_str("ieth");
    } else {
        out.push_str("th");
    }
    Ok(out)
}

/// `num2words(n, to='year')` for 0 ≤ n.
pub fn year(n: u64) -> String {
    let (high, low) = (n / 100, n % 100);
    if high == 0 || (high % 10 == 0 && low < 10) || high >= 100 {
        return cardinal_u64(n);
    }
    let mut out = cardinal_u64(high);
    out.push(' ');
    match low {
        0 => out.push_str("hundred"),
        1..=9 => {
            out.push_str("oh-");
            out.push_str(ONES[low as usize]);
        }
        _ => push_below_thousand(&mut out, low as u32),
    }
    out
}

/// `num2words(float(text))` for text Python's `float` accepts and that is non-negative.
pub fn float(text: &str) -> Result<String, Error> {
    let v: f64 = text.parse().map_err(|_| Error::TooLarge)?;
    if !v.is_finite() {
        return Err(Error::TooLarge);
    }
    if v.fract() == 0.0 {
        let mut out = String::new();
        push_integral_float(&mut out, v)?;
        return Ok(out);
    }
    let whole = v.trunc();
    let mut out = cardinal_u64(whole as u64);
    let sci = format!("{v:e}");
    let (mantissa, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    let mantissa_digits = mantissa.bytes().filter(u8::is_ascii_digit).count() as i32;
    let precision = (mantissa_digits - 1 - exp).max(0) as usize;
    if precision > 308 {
        return Err(Error::TooLarge);
    }
    let post = (v - whole) * format!("1e{precision}").parse::<f64>().expect("valid float");
    let rounded = post.round_ties_even();
    let post = if (rounded - post).abs() < 0.01 { rounded } else { post.floor() };
    let post = exact_integer_digits(post);
    out.push_str(" point");
    let pad = precision.saturating_sub(post.len());
    for d in std::iter::repeat_n(b'0', pad).chain(post.bytes()).take(precision) {
        out.push(' ');
        out.push_str(ONES[(d - b'0') as usize]);
    }
    Ok(out)
}

/// Names an integral float the way num2words does: Python's float `divmod` by each power of a thousand.
fn push_integral_float(out: &mut String, v: f64) -> Result<(), Error> {
    if v < 1000.0 {
        push_below_thousand(out, v as u32);
        return Ok(());
    }
    if v < 9007199254740992.0 {
        push_cardinal_digits(out, (v as u64).to_string().as_bytes());
        return Ok(());
    }
    let digits = exact_integer_digits(v).len();
    if digits > MAX_DIGITS {
        return Err(Error::TooLarge);
    }
    let scale = (digits - 1) / 3;
    let unit: f64 = format!("1e{}", 3 * scale).parse().expect("valid float");
    let rem = v % unit;
    let quotient = (v - rem) / unit;
    let mut div = quotient.floor();
    if quotient - div > 0.5 {
        div += 1.0;
    }
    push_integral_float(out, div)?;
    out.push(' ');
    out.push_str(SCALES[scale - 1]);
    if rem != 0.0 {
        out.push_str(if rem < 100.0 { " and " } else { ", " });
        push_integral_float(out, rem)?;
    }
    Ok(())
}

/// The exact decimal digits of a finite, non-negative, integral float.
fn exact_integer_digits(v: f64) -> String {
    debug_assert!(v.is_finite() && v >= 0.0 && v.fract() == 0.0);
    if v < 18446744073709551616.0 {
        return (v as u64).to_string();
    }
    let bits = v.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1075;
    let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
    // Little-endian base-10^9 limbs.
    let mut limbs: Vec<u32> = vec![(mantissa % 1_000_000_000) as u32, (mantissa / 1_000_000_000) as u32];
    for _ in 0..exponent {
        let mut carry = 0u64;
        for limb in &mut limbs {
            let x = u64::from(*limb) * 2 + carry;
            *limb = (x % 1_000_000_000) as u32;
            carry = x / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    while limbs.len() > 1 && limbs.last() == Some(&0) {
        limbs.pop();
    }
    let mut out = limbs.last().expect("non-empty").to_string();
    for limb in limbs.iter().rev().skip(1) {
        out.push_str(&format!("{limb:09}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples() {
        assert_eq!(cardinal("0").unwrap(), "zero");
        assert_eq!(cardinal("1100").unwrap(), "one thousand, one hundred");
        assert_eq!(cardinal("1000050").unwrap(), "one million and fifty");
        assert_eq!(ordinal("22").unwrap(), "twenty-second");
        assert_eq!(ordinal("40").unwrap(), "fortieth");
        assert_eq!(year(1906), "nineteen oh-six");
        assert_eq!(year(2005), "two thousand and five");
        assert_eq!(float("0.07").unwrap(), "zero point zero seven");
        assert!(cardinal(&"1".repeat(307)).is_err());
        assert_eq!(exact_integer_digits(1e23), "99999999999999991611392");
    }
}
