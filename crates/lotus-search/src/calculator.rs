#[derive(Clone, Debug, PartialEq)]
pub struct Calculation {
    pub value: String,
}

pub fn calculate(query: &str) -> Option<Calculation> {
    let expression = normalized_expression(query)?;
    if !identifiers_are_supported(&expression) {
        return None;
    }
    let mut namespace = |name: &str, arguments: Vec<f64>| match (name, arguments.as_slice())
    {
        ("sqrt", [value]) => Some(value.sqrt()),
        ("exp", [value]) => Some(value.exp()),
        ("ln", [value]) => Some(value.ln()),
        ("atan2", [y, x]) => Some(y.atan2(*x)),
        ("pi", []) => Some(std::f64::consts::PI),
        ("e", []) => Some(std::f64::consts::E),
        _ => None,
    };
    let value = fasteval::ez_eval(&expression, &mut namespace).ok()?;
    value.is_finite().then(|| Calculation {
        value: format_value(value),
    })
}

fn identifiers_are_supported(expression: &str) -> bool {
    expression
        .split(|character: char| !character.is_ascii_alphabetic() && character != '_')
        .filter(|identifier| !identifier.is_empty())
        .all(|identifier| {
            matches!(
                identifier,
                "sqrt"
                    | "abs"
                    | "exp"
                    | "ln"
                    | "log"
                    | "sin"
                    | "cos"
                    | "tan"
                    | "asin"
                    | "acos"
                    | "atan"
                    | "atan2"
                    | "sinh"
                    | "cosh"
                    | "tanh"
                    | "asinh"
                    | "acosh"
                    | "atanh"
                    | "floor"
                    | "ceil"
                    | "round"
                    | "sign"
                    | "min"
                    | "max"
                    | "int"
                    | "pi"
                    | "e"
            )
        })
}

fn normalized_expression(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() || trimmed.starts_with('>') || trimmed.len() > 160 {
        return None;
    }

    let explicit = trimmed.strip_prefix('=').map(str::trim);
    let expression = explicit.unwrap_or(trimmed);
    let has_math_shape = explicit.is_some()
        || expression
            .chars()
            .any(|character| "+-*/%^()×÷".contains(character))
        || expression
            .split_whitespace()
            .any(|part| matches!(part, "x" | "X"));
    if !has_math_shape || !expression.bytes().any(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let expression = expression.replace('×', "*").replace('÷', "/");
    Some(normalize_word_multiply(&expression))
}

fn normalize_word_multiply(expression: &str) -> String {
    expression
        .split_whitespace()
        .map(|part| {
            if matches!(part, "x" | "X") {
                "*"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_value(value: f64) -> String {
    let integer_tolerance = f64::EPSILON * value.abs().max(1.0) * 8.0;
    if value.fract().abs() <= integer_tolerance && value.abs() < 1.0e15 {
        return format!("{value:.0}");
    }

    if value == 0.0 || (1.0e-9..1.0e12).contains(&value.abs()) {
        return format!("{value:.10}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned();
    }

    let scientific = format!("{value:.8e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    format!(
        "{}e{exponent}",
        mantissa.trim_end_matches('0').trim_end_matches('.')
    )
}

#[cfg(test)]
mod tests {
    use super::{Calculation, calculate};

    #[test]
    fn calculator_requires_math_input_and_formats_useful_results() {
        let cases = [
            ("42", None),
            ("7zip", None),
            ("12 * (4 + 3)", Some("84")),
            ("sqrt(81)", Some("9")),
            ("= 1 / 8", Some("0.125")),
            ("2 × 6", Some("12")),
            ("12 x 5", Some("60")),
            ("Xbox", None),
        ];

        for (query, expected) in cases {
            assert_eq!(
                calculate(query),
                expected.map(|value| Calculation {
                    value: value.to_owned(),
                }),
                "{query}"
            );
        }
    }
}
