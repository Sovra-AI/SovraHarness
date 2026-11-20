//! A Sovra code plugin: exact date sums for the helper, which otherwise guesses.
//!
//! Sovra calls `sovra_call` with a tool name and its JSON arguments and reads back
//! `{"text": …}` or `{"error": …}`. The module imports nothing, so in Sovra's
//! sealed box it cannot reach files, programs or the internet. Memory is never
//! handed back: Sovra starts every call from a fresh copy of the module.

use serde_json::{Value, json};

#[unsafe(no_mangle)]
pub extern "C" fn sovra_alloc(length: usize) -> *mut u8 {
    Vec::<u8>::with_capacity(length).leak().as_mut_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn sovra_call(
    name: *const u8,
    name_length: usize,
    arguments: *const u8,
    arguments_length: usize,
) -> u64 {
    // SAFETY: Sovra wrote these bytes into buffers `sovra_alloc` returned.
    let (name, arguments) = unsafe {
        (
            std::slice::from_raw_parts(name, name_length),
            std::slice::from_raw_parts(arguments, arguments_length),
        )
    };
    let reply = match answer(name, arguments) {
        Ok(text) => json!({ "text": text }),
        Err(error) => json!({ "error": error }),
    };
    let bytes = serde_json::to_vec(&reply).unwrap_or_default().leak();
    ((bytes.as_ptr() as u64) << 32) | bytes.len() as u64
}

fn answer(name: &[u8], arguments: &[u8]) -> Result<String, String> {
    let arguments: Value =
        serde_json::from_slice(arguments).map_err(|_| "The arguments were not JSON.".to_string())?;
    match name {
        b"days_between" => {
            let from = date(&arguments, "from")?;
            let to = date(&arguments, "to")?;
            let days = to.days() - from.days();
            Ok(format!(
                "From {from} to {to} is {} day{}{}.",
                days.abs(),
                if days.abs() == 1 { "" } else { "s" },
                if days < 0 { " earlier" } else { "" }
            ))
        }
        b"weekday" => {
            let day = date(&arguments, "date")?;
            Ok(format!("{day} is a {}.", day.weekday()))
        }
        b"add_days" => {
            let day = date(&arguments, "date")?;
            let count = arguments
                .get("days")
                .and_then(Value::as_i64)
                .filter(|count| count.abs() <= 3_650_000)
                .ok_or("Give \"days\" as a whole number, like 30 or -7.")?;
            let later = Date::from_days(day.days() + count);
            Ok(format!(
                "{day} plus {count} day{} is {later}, a {}.",
                if count.abs() == 1 { "" } else { "s" },
                later.weekday()
            ))
        }
        _ => Err("This plugin has no tool by that name.".into()),
    }
}

#[derive(Clone, Copy)]
struct Date {
    year: i64,
    month: i64,
    day: i64,
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const MONTHS: [&str; 12] = [
            "January", "February", "March", "April", "May", "June", "July", "August",
            "September", "October", "November", "December",
        ];
        let month = usize::try_from(self.month - 1).unwrap_or(0);
        write!(f, "{} {} {}", self.day, MONTHS[month], self.year)
    }
}

fn date(arguments: &Value, field: &str) -> Result<Date, String> {
    let text = arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Give \"{field}\" as a date like 2026-12-25."))?;
    Date::parse(text).ok_or_else(|| format!("\"{text}\" is not a real date. Use the form 2026-12-25."))
}

impl Date {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().splitn(3, '-');
        let year: i64 = parts.next()?.parse().ok()?;
        let month: i64 = parts.next()?.parse().ok()?;
        let day: i64 = parts.next()?.parse().ok()?;
        let parsed = Self { year, month, day };
        ((1..=9999).contains(&year)
            && (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day))
        .then_some(parsed)
    }

    /// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's
    /// `days_from_civil`).
    fn days(self) -> i64 {
        let year = if self.month <= 2 { self.year - 1 } else { self.year };
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = (self.month + 9) % 12;
        let day_of_year = (153 * month + 2) / 5 + self.day - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    fn from_days(days: i64) -> Self {
        let days = days + 719_468;
        let era = days.div_euclid(146_097);
        let day_of_era = days - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month + 2) / 5 + 1;
        let month = if month < 10 { month + 3 } else { month - 9 };
        Self {
            year: year_of_era + era * 400 + i64::from(month <= 2),
            month,
            day,
        }
    }

    fn weekday(self) -> &'static str {
        // 1970-01-01 was a Thursday.
        const NAMES: [&str; 7] = [
            "Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday",
        ];
        NAMES[usize::try_from(self.days().rem_euclid(7)).unwrap_or(0)]
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
