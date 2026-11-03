use time::{OffsetDateTime, UtcOffset, macros::format_description};

/// The `<t:unix:style>` styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimestampStyle {
    /// `t`: 4:20 PM
    ShortTime,
    /// `T`: 4:20:30 PM
    LongTime,
    /// `d`: 11/03/2026
    ShortDate,
    /// `D`: November 3, 2026
    LongDate,
    /// `f`, the default: November 3, 2026 4:20 PM
    ShortDateTime,
    /// `F`: Tuesday, November 3, 2026 4:20 PM
    LongDateTime,
    /// `R`: 2 hours ago
    Relative,
}

impl TimestampStyle {
    pub fn from_char(c: char) -> Option<Self> {
        Some(match c {
            't' => Self::ShortTime,
            'T' => Self::LongTime,
            'd' => Self::ShortDate,
            'D' => Self::LongDate,
            'f' => Self::ShortDateTime,
            'F' => Self::LongDateTime,
            'R' => Self::Relative,
            _ => return None,
        })
    }
}

/// Formats a timestamp in `offset`, with times in 24-hour form when `hour24` is set.
/// `now` is only used by the relative style.
pub fn format_timestamp(
    unix: i64,
    style: TimestampStyle,
    now: i64,
    offset: UtcOffset,
    hour24: bool,
) -> String {
    let Ok(at) = OffsetDateTime::from_unix_timestamp(unix) else {
        return unix.to_string();
    };
    let at = at.to_offset(offset);
    let time_short = if hour24 {
        at.format(format_description!("[hour]:[minute]"))
    } else {
        at.format(format_description!(
            "[hour repr:12 padding:none]:[minute] [period]"
        ))
    };
    let time_long = if hour24 {
        at.format(format_description!("[hour]:[minute]:[second]"))
    } else {
        at.format(format_description!(
            "[hour repr:12 padding:none]:[minute]:[second] [period]"
        ))
    };
    let date_long = at.format(format_description!(
        "[month repr:long] [day padding:none], [year]"
    ));
    let result = match style {
        TimestampStyle::ShortTime => time_short,
        TimestampStyle::LongTime => time_long,
        TimestampStyle::ShortDate => at.format(format_description!("[month]/[day]/[year]")),
        TimestampStyle::LongDate => date_long,
        TimestampStyle::ShortDateTime => {
            date_long.and_then(|d| time_short.map(|t| format!("{d} {t}")))
        }
        TimestampStyle::LongDateTime => at
            .format(format_description!("[weekday], "))
            .and_then(|w| date_long.and_then(|d| time_short.map(|t| format!("{w}{d} {t}")))),
        TimestampStyle::Relative => return relative(unix - now),
    };
    result.unwrap_or_else(|_| unix.to_string())
}

fn relative(delta: i64) -> String {
    let future = delta > 0;
    let secs = delta.unsigned_abs();
    let (n, unit) = match secs {
        0..45 => {
            return if future {
                "in a few seconds".into()
            } else {
                "a few seconds ago".into()
            };
        }
        45..2_700 => (secs.div_ceil(60).max(1), "minute"),
        2_700..79_200 => ((secs + 1_800) / 3_600, "hour"),
        79_200..2_246_400 => ((secs + 43_200) / 86_400, "day"),
        2_246_400..27_648_000 => ((secs + 1_296_000) / 2_592_000, "month"),
        _ => ((secs + 15_768_000) / 31_536_000, "year"),
    };
    let n = n.max(1);
    let unit = if n == 1 {
        unit.to_owned()
    } else {
        format!("{unit}s")
    };
    let amount = if n == 1 {
        if unit == "hour" {
            "an".to_owned()
        } else {
            "a".to_owned()
        }
    } else {
        n.to_string()
    };
    if future {
        format!("in {amount} {unit}")
    } else {
        format!("{amount} {unit} ago")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z

    #[test]
    fn absolute_styles() {
        let utc = UtcOffset::UTC;
        assert_eq!(
            format_timestamp(T, TimestampStyle::ShortTime, T, utc, false),
            "10:13 PM"
        );
        assert_eq!(
            format_timestamp(T, TimestampStyle::LongTime, T, utc, true),
            "22:13:20"
        );
        assert_eq!(
            format_timestamp(T, TimestampStyle::ShortDate, T, utc, false),
            "11/14/2023"
        );
        assert_eq!(
            format_timestamp(T, TimestampStyle::LongDateTime, T, utc, false),
            "Tuesday, November 14, 2023 10:13 PM"
        );
    }

    #[test]
    fn relative_style() {
        let utc = UtcOffset::UTC;
        let r = |d| format_timestamp(T, TimestampStyle::Relative, T - d, utc, false);
        assert_eq!(r(-10), "a few seconds ago");
        assert_eq!(r(-3_600 * 2), "2 hours ago");
        assert_eq!(r(-3_600), "an hour ago");
        assert_eq!(r(86_400 * 3), "in 3 days");
    }
}
