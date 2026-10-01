use std::time::Duration;

use anyhow::{Result, bail};

pub(crate) fn parse_duration(value: &str) -> Result<Duration> {
    let value = value.trim();
    let digit_end = value.bytes().take_while(u8::is_ascii_digit).count();
    if digit_end == 0 {
        bail!("`{value}` is not a duration; use 500ms, 90s, 5m or 1h");
    }

    let (digits, unit) = value.split_at(digit_end);
    let amount = digits
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("duration `{value}` is too long"))?;
    let milliseconds_per_unit = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "" if amount == 0 => return Ok(Duration::ZERO),
        "" => bail!("`{value}` needs a unit: ms, s, m or h"),
        _ if unit
            .chars()
            .all(|character| character.is_ascii_alphabetic()) =>
        {
            bail!("`{value}` uses unknown unit `{unit}`; use ms, s, m or h")
        }
        _ => bail!("`{value}` is not a duration; use 500ms, 90s, 5m or 1h"),
    };
    amount
        .checked_mul(milliseconds_per_unit)
        .map(Duration::from_millis)
        .ok_or_else(|| anyhow::anyhow!("duration `{value}` is too long"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_unit_and_unitless_zero() {
        for (value, expected) in [
            ("500ms", Duration::from_millis(500)),
            ("90s", Duration::from_secs(90)),
            ("5m", Duration::from_secs(300)),
            ("1h", Duration::from_secs(3_600)),
            ("0", Duration::ZERO),
        ] {
            assert_eq!(parse_duration(value).unwrap(), expected, "{value}");
        }
    }

    #[test]
    fn rejects_values_outside_the_grammar() {
        for (value, expected) in [
            ("15", "needs a unit: ms, s, m or h"),
            ("5d", "unknown unit `d`"),
            ("0.5s", "is not a duration"),
            ("", "is not a duration"),
            ("18446744073709551615h", "is too long"),
        ] {
            let error = parse_duration(value).unwrap_err().to_string();
            assert!(error.contains(expected), "{value}: {error}");
        }
    }
}
