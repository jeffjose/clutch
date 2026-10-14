//! Sums and conversions typed into the box: "12*7", "2^10 / 3",
//! "sqrt(2)", "5 km in mi", "350f to c", "2.5 gb in mb". The answer is
//! the first row, and Enter copies it.

/// The answer to what was typed, as it is to be shown and copied, if it
/// is a sum or a conversion. A number alone is neither.
pub fn answer(typed: &str) -> Option<String> {
    let typed = typed.trim();
    convert(typed).or_else(|| {
        let mut sum = Sum { letters: typed.chars().filter(|c| !c.is_whitespace()).collect(), at: 0, worked: false };
        let value = sum.expression()?;
        (sum.at == sum.letters.len() && sum.worked && value.is_finite()).then(|| said(value))
    })
}

/// A number as a person would write it: no long tail of digits, and no ".0".
fn said(value: f64) -> String {
    let rounded = (value * 1e10).round() / 1e10;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        let text = format!("{rounded:.10}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Reading a sum, a letter at a time, the usual way round: powers before
/// times and over, those before plus and minus.
struct Sum {
    letters: Vec<char>,
    at: usize,
    /// Whether anything was done: an operator or a function, and not a number alone.
    worked: bool,
}

impl Sum {
    fn peek(&self) -> Option<char> {
        self.letters.get(self.at).copied()
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(sign @ ('+' | '-')) = self.peek() {
            self.at += 1;
            self.worked = true;
            let next = self.term()?;
            value = if sign == '+' { value + next } else { value - next };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        while let Some(sign @ ('*' | '/' | '%' | 'x' | '×' | '÷')) = self.peek() {
            self.at += 1;
            self.worked = true;
            let next = self.power()?;
            value = match sign {
                '/' | '÷' => value / next,
                '%' => value % next,
                _ => value * next,
            };
        }
        Some(value)
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.peek() == Some('^') {
            self.at += 1;
            self.worked = true;
            // Powers go from the right: 2^3^2 is 2^9.
            return Some(base.powf(self.power()?));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.peek() == Some('-') {
            self.at += 1;
            return Some(-self.unary()?);
        }
        self.atom()
    }

    fn atom(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if c == '(' {
            self.at += 1;
            let value = self.expression()?;
            (self.peek() == Some(')')).then(|| self.at += 1)?;
            return Some(value);
        }
        if c.is_ascii_digit() || c == '.' {
            let from = self.at;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == ',') {
                self.at += 1;
            }
            let number: String = self.letters[from..self.at].iter().filter(|c| **c != ',').collect();
            return number.parse().ok();
        }
        if c.is_ascii_alphabetic() {
            let from = self.at;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.at += 1;
            }
            let word: String = self.letters[from..self.at].iter().collect::<String>().to_lowercase();
            self.worked = true;
            match word.as_str() {
                "pi" => return Some(std::f64::consts::PI),
                "e" => return Some(std::f64::consts::E),
                _ => {}
            }
            let of = self.atom()?;
            return Some(match word.as_str() {
                "sqrt" => of.sqrt(),
                "abs" => of.abs(),
                "ln" => of.ln(),
                "log" => of.log10(),
                "sin" => of.to_radians().sin(),
                "cos" => of.to_radians().cos(),
                "tan" => of.to_radians().tan(),
                "round" => of.round(),
                "floor" => of.floor(),
                "ceil" => of.ceil(),
                _ => return None,
            });
        }
        None
    }
}

/// The units there are, each with how many of the kind's first unit it is.
const UNITS: &[(&str, &[&str], f64)] = &[
    ("length", &["m", "meter", "meters", "metre", "metres"], 1.0),
    ("length", &["km", "kilometer", "kilometers"], 1000.0),
    ("length", &["cm", "centimeter", "centimeters"], 0.01),
    ("length", &["mm", "millimeter", "millimeters"], 0.001),
    ("length", &["mi", "mile", "miles"], 1609.344),
    ("length", &["yd", "yard", "yards"], 0.9144),
    ("length", &["ft", "foot", "feet"], 0.3048),
    ("length", &["in", "inch", "inches"], 0.0254),
    ("mass", &["kg", "kilogram", "kilograms", "kilo", "kilos"], 1.0),
    ("mass", &["g", "gram", "grams"], 0.001),
    ("mass", &["mg", "milligram", "milligrams"], 0.000001),
    ("mass", &["lb", "lbs", "pound", "pounds"], 0.45359237),
    ("mass", &["oz", "ounce", "ounces"], 0.028349523125),
    ("mass", &["st", "stone", "stones"], 6.35029318),
    ("volume", &["l", "liter", "liters", "litre", "litres"], 1.0),
    ("volume", &["ml", "milliliter", "milliliters"], 0.001),
    ("volume", &["gal", "gallon", "gallons"], 3.785411784),
    ("volume", &["cup", "cups"], 0.2365882365),
    ("volume", &["floz"], 0.0295735295625),
    ("time", &["s", "sec", "second", "seconds"], 1.0),
    ("time", &["min", "minute", "minutes"], 60.0),
    ("time", &["h", "hr", "hour", "hours"], 3600.0),
    ("time", &["day", "days"], 86400.0),
    ("time", &["week", "weeks"], 604800.0),
    ("time", &["year", "years"], 31557600.0),
    ("data", &["b", "byte", "bytes"], 1.0),
    ("data", &["kb"], 1e3),
    ("data", &["mb"], 1e6),
    ("data", &["gb"], 1e9),
    ("data", &["tb"], 1e12),
    ("data", &["kib"], 1024.0),
    ("data", &["mib"], 1048576.0),
    ("data", &["gib"], 1073741824.0),
    ("data", &["tib"], 1099511627776.0),
    ("speed", &["mps"], 1.0),
    ("speed", &["kph", "kmh"], 1.0 / 3.6),
    ("speed", &["mph"], 0.44704),
    ("speed", &["knot", "knots", "kn"], 0.514444),
];

fn unit(word: &str) -> Option<(&'static str, f64)> {
    let word = word.to_lowercase();
    UNITS.iter().find(|(_, names, _)| names.contains(&word.as_str())).map(|(kind, _, size)| (*kind, *size))
}

/// Temperatures, which are not so many of another but so many from it.
fn to_kelvin(value: f64, scale: &str) -> Option<f64> {
    Some(match scale {
        "c" | "celsius" => value + 273.15,
        "f" | "fahrenheit" => (value - 32.0) * 5.0 / 9.0 + 273.15,
        "k" | "kelvin" => value,
        _ => return None,
    })
}

fn from_kelvin(kelvin: f64, scale: &str) -> Option<f64> {
    Some(match scale {
        "c" | "celsius" => kelvin - 273.15,
        "f" | "fahrenheit" => (kelvin - 273.15) * 9.0 / 5.0 + 32.0,
        "k" | "kelvin" => kelvin,
        _ => return None,
    })
}

/// "5 km in mi", "5km to mi", "350 f in c": the number, the unit it is
/// in, and the one wanted.
fn convert(typed: &str) -> Option<String> {
    let lower = typed.to_lowercase();
    let (from, to) = lower.split_once(" in ").or_else(|| lower.split_once(" to "))?;
    let (to, from) = (to.trim().trim_start_matches('°'), from.trim());
    // Where the number ends and its unit begins.
    let split = from.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == ',' || c.is_whitespace()))?;
    let value: f64 = from[..split].trim().replace(',', "").parse().ok()?;
    let from_unit = from[split..].trim().trim_start_matches('°');
    let result = match (unit(from_unit), unit(to)) {
        (Some((kind, size)), Some((wanted, into))) if kind == wanted => value * size / into,
        _ => from_kelvin(to_kelvin(value, from_unit)?, to)?,
    };
    Some(format!("{} {to}", said(result)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums() {
        assert_eq!(answer("12*7").as_deref(), Some("84"));
        assert_eq!(answer("2 + 3 * 4").as_deref(), Some("14"));
        assert_eq!(answer("(2+3)*4").as_deref(), Some("20"));
        assert_eq!(answer("2^10").as_deref(), Some("1024"));
        assert_eq!(answer("10/4").as_deref(), Some("2.5"));
        assert_eq!(answer("sqrt(16)").as_deref(), Some("4"));
        assert_eq!(answer("-3+1").as_deref(), Some("-2"));
        assert_eq!(answer("1/3").as_deref(), Some("0.3333333333"));
    }

    #[test]
    fn what_is_not_a_sum() {
        assert_eq!(answer("2025"), None);
        assert_eq!(answer("firefox"), None);
        assert_eq!(answer("12*"), None);
        assert_eq!(answer("1/0"), None);
    }

    #[test]
    fn conversions() {
        assert_eq!(answer("5 km in mi").as_deref(), Some("3.1068559612 mi"));
        assert_eq!(answer("1 mi to km").as_deref(), Some("1.609344 km"));
        assert_eq!(answer("212f in c").as_deref(), Some("100 c"));
        assert_eq!(answer("2.5 gb in mb").as_deref(), Some("2500 mb"));
        assert_eq!(answer("90 min in h").as_deref(), Some("1.5 h"));
        assert_eq!(answer("5 km in kg"), None);
    }
}
