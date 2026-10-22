//! PDF-Datumsangaben ("D:YYYYMMDDHHmmSSZ") ohne externe Datums-Crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// Aktuelle Zeit (UTC) im PDF-Datumsformat.
pub fn pdf_date_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    pdf_date_from_unix(secs)
}

pub fn pdf_date_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Tage seit 1970-01-01 -> (Jahr, Monat, Tag). Algorithmus von Howard Hinnant
/// ("days_from_civil" rückwärts), gültig für den gregorianischen Kalender.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(pdf_date_from_unix(0), "D:19700101000000Z");
        assert_eq!(pdf_date_from_unix(951_782_400), "D:20000229000000Z"); // Schalttag
        assert_eq!(pdf_date_from_unix(1_792_665_045), "D:20261022103045Z");
    }
}
