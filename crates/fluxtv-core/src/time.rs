//! Minimal date/time helpers (avoids pulling a full date library into the core).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix time in seconds.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse an XMLTV timestamp such as `20260922110000 +0000` into Unix seconds.
///
/// The spec allows truncated forms (`YYYYMMDDhhmm`, `YYYYMMDD`) and a missing offset,
/// in which case UTC is assumed.
pub fn parse_xmltv_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let (digits, rest) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    if digits.len() < 8 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> {
        digits.get(a..b).map(|x| x.parse().ok()).unwrap_or(Some(0))
    };
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    let (h, mi, se) = (num(8, 10)?, num(10, 12)?, num(12, 14)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut ts = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se;

    let offset = rest.trim();
    if let Some(sign) = offset.chars().next().filter(|c| *c == '+' || *c == '-') {
        let o: String = offset[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if o.len() >= 4 {
            let oh: i64 = o[0..2].parse().ok()?;
            let om: i64 = o[2..4].parse().ok()?;
            let secs = oh * 3600 + om * 60;
            // Local time = UTC + offset, so UTC = local - offset.
            ts -= if sign == '+' { secs } else { -secs };
        }
    }
    Some(ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(parse_xmltv_time("19700101000000 +0000"), Some(0));
    }

    #[test]
    fn with_offsets() {
        // 2026-09-22 11:00:00 UTC
        let utc = parse_xmltv_time("20260922110000 +0000").unwrap();
        assert_eq!(utc, 1_790_074_800);
        assert_eq!(parse_xmltv_time("20260922130000 +0200"), Some(utc));
        assert_eq!(parse_xmltv_time("20260922060000 -0500"), Some(utc));
        assert_eq!(parse_xmltv_time("20260922110000"), Some(utc));
        assert_eq!(parse_xmltv_time("202609221100 +0000"), Some(utc));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_xmltv_time(""), None);
        assert_eq!(parse_xmltv_time("2026"), None);
        assert_eq!(parse_xmltv_time("20261301000000"), None);
    }
}
