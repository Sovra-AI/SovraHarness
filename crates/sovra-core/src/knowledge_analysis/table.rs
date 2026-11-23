//! Infer table profiles from observed column traits, retaining ambiguous matches.
//! Quoted fields and embedded newlines follow CSV rules. Never sort source rows in place.

use super::{Finding, FindingKind, measurement::Quantity, topology};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use time::{Date, Month, OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) fn timestamp(value: &str) -> Option<i64> {
    if let Ok(at) = OffsetDateTime::parse(value, &Rfc3339) {
        return Some(at.unix_timestamp());
    }
    if value.len() != 10 || !value.is_ascii() {
        return None;
    }
    let mut parts = value.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if year.len() != 4 || month.len() != 2 || day.len() != 2 || parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(
        year.parse().ok()?,
        Month::try_from(month.parse::<u8>().ok()?).ok()?,
        day.parse().ok()?,
    )
    .ok()
    .map(|d| d.midnight().assume_utc().unix_timestamp())
}

pub(super) struct Table {
    pub header: csv::StringRecord,
    pub rows: Vec<csv::StringRecord>,
    first: u32,
    limited: bool,
}
pub(super) fn parse(text: &str) -> Option<Table> {
    let mut iter = text.lines().enumerate().peekable();
    let (header_at, first) =
        iter.find(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with("```"))?;
    let mut rows = Vec::new();
    let header = if first.contains('|')
        && iter.peek().is_some_and(|(_, line)| {
            line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        }) {
        let split = |line: &str| {
            csv::StringRecord::from_iter(line.trim().trim_matches('|').split('|').map(str::trim))
        };
        let header = split(first);
        iter.next();
        for (at, line) in iter.take_while(|(_, line)| line.contains('|')).take(1025) {
            let mut row = split(line);
            if row.len() != header.len() {
                return None;
            }
            row.set_position(Some(csv::Position::new().set_line((at + 1) as u64).clone()));
            rows.push(row);
        }
        header
    } else {
        let delimiter = if first.contains('\t') {
            b'\t'
        } else if first.contains(',') {
            b','
        } else {
            return None;
        };
        let start = first.as_ptr().addr() - text.as_ptr().addr();
        let mut body = &text[start..];
        if header_at > 0
            && let Some((data, _)) = body.split_once("\n```")
        {
            body = data;
        }
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .trim(csv::Trim::All)
            .from_reader(body.as_bytes());
        let header = reader.headers().ok()?.clone();
        for row in reader.records().take(1025) {
            let mut row = row.ok()?;
            if let Some(mut position) = row.position().cloned() {
                position.set_line(position.line() + header_at as u64);
                row.set_position(Some(position));
            }
            rows.push(row);
        }
        header
    };
    if !(2..=64).contains(&header.len()) || header.iter().any(str::is_empty) || rows.is_empty() {
        return None;
    }
    let limited = rows.len() > 1024;
    rows.truncate(1024);
    Some(Table {
        header,
        rows,
        first: u32::try_from(header_at + 1).ok()?,
        limited,
    })
}
pub(super) fn inspect(
    table: &Table,
    options: &super::AnalysisOptions,
    output: &mut Vec<Finding>,
    quantities: &mut Vec<super::MeasuredQuantity>,
) {
    let window_seconds = options.window_seconds;
    let Table {
        header,
        rows,
        first,
        limited,
    } = table;
    let last = rows
        .last()
        .and_then(csv::StringRecord::position)
        .and_then(|p| u32::try_from(p.line()).ok())
        .unwrap_or(1);
    let mut add = |kind, detail: String| {
        output.push(Finding {
            kind,
            from: *first,
            to: last,
            detail,
        });
    };
    let column = |names: &[&str]| {
        header
            .iter()
            .position(|h| names.iter().any(|name| h.eq_ignore_ascii_case(name)))
    };
    let dates = column(&["date", "time", "timestamp", "event_time", "valid_at"]);
    let value = column(&["value", "reading", "measurement"]);
    let unit = column(&["unit", "units"]);
    let mut profiles = Vec::new();
    let numeric: Vec<usize> = header
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            rows.iter()
                .all(|r| r[*i].parse::<f64>().is_ok_and(f64::is_finite))
        })
        .map(|(i, _)| i)
        .collect();
    if dates.is_some_and(|col| rows.iter().all(|r| timestamp(&r[col]).is_some())) {
        profiles.push("dated observations");
    }
    if value
        .zip(unit)
        .is_some_and(|(v, u)| rows.iter().all(|r| Quantity::parse(&r[v], &r[u]).is_some()))
    {
        profiles.push("measured quantities");
    }
    let axes: Vec<_> = numeric
        .iter()
        .copied()
        .filter(|i| {
            let name = header[*i].to_ascii_lowercase();
            matches!(name.as_str(), "x" | "y" | "z")
                || name.strip_prefix("dim").is_some_and(|tail| {
                    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
                })
        })
        .collect();
    if (2..=16).contains(&axes.len()) {
        profiles.push("coordinate samples");
    }
    let blanks: usize = rows
        .iter()
        .map(|r| r.iter().filter(|v| v.is_empty()).count())
        .sum();
    let mut matches = Vec::new();
    for rule in &options.profiles {
        if rule
            .required_columns
            .iter()
            .all(|name| column(&[name]).is_some())
            && rule
                .numeric_columns
                .iter()
                .all(|name| column(&[name]).is_some_and(|i| numeric.contains(&i)))
        {
            matches.push(rule.name.as_str());
        }
    }
    let matching_profiles = match matches.as_slice() {
        [] => String::new(),
        [name] => format!(" · profile {name}"),
        _ => format!(" · equally matching profiles: {}", matches.join(", ")),
    };
    let names = if profiles.is_empty() {
        "table of unknown type".into()
    } else {
        profiles.join(", ")
    };
    add(
        FindingKind::Profile,
        format!(
            "{names} · {} row{}, {} columns, {} numeric column{}, {blanks} empty cell{}{matching_profiles}{}",
            rows.len(),
            if rows.len() == 1 { "" } else { "s" },
            header.len(),
            numeric.len(),
            if numeric.len() == 1 { "" } else { "s" },
            if blanks == 1 { "" } else { "s" },
            if *limited {
                " in the first 1,024 rows"
            } else {
                ""
            }
        ),
    );
    if (2..=16).contains(&axes.len()) {
        let points: Vec<Vec<f64>> = rows
            .iter()
            .take(topology::MAX_POINTS)
            .map(|r| axes.iter().filter_map(|i| r[*i].parse().ok()).collect())
            .collect();
        if let Ok(bars) = topology::persistence(&points) {
            let axes = axes
                .iter()
                .map(|i| &header[*i])
                .collect::<Vec<_>>()
                .join(", ");
            let separation = bars.component_deaths.last().copied().unwrap_or(0.);
            let mut detail = format!(
                "Euclidean distances on {axes}, first {} points. All components join at distance {separation:.5}. {} positive-length loops.",
                bars.points,
                bars.loops.len()
            );
            if let Some(bar) = bars.loops.first() {
                use std::fmt::Write as _;
                let _ = write!(
                    detail,
                    " Longest loop persists from {:.5} to {:.5}.",
                    bar.birth, bar.death
                );
            }
            let mut scales = vec![0., separation];
            if let Some(bar) = bars.loops.first() {
                scales.push(bar.birth + (bar.death - bar.birth) / 2.0);
                scales.push(bar.death);
            } else if let Some(&first) = bars.component_deaths.first() {
                scales.push(first);
            }
            scales.sort_by(f64::total_cmp);
            scales.dedup();
            for scale in scales {
                use std::fmt::Write as _;
                let (components, loops) = bars.at(scale);
                let _ = write!(
                    detail,
                    " At {scale:.5}: {components} components, {loops} loops."
                );
            }
            add(FindingKind::Topology, detail);
        }
    }
    let Some((v, u)) = value.zip(unit) else {
        return;
    };
    let instrument = column(&["instrument", "sensor"]);
    let calibration = column(&["calibration_date", "calibrated_at"]);
    let reference = column(&["reference", "calibration_reference"]);
    let uncertainty = column(&["uncertainty"]);
    let reference_value = column(&["reference_value", "expected_value"]);
    let reference_unit = column(&["reference_unit", "expected_unit"]);
    let tolerance = column(&["tolerance"]);
    let mut windows = BTreeMap::new();
    for row in rows {
        let Some(q) = Quantity::parse(&row[v], &row[u]) else {
            continue;
        };
        if let Some(at) = dates.and_then(|i| timestamp(&row[i])) {
            let window = at.div_euclid(i64::from(window_seconds)) * i64::from(window_seconds);
            let series = instrument.map_or("", |i| &row[i]);
            let entry = windows
                .entry((window, q.dimension, q.unit, series))
                .or_insert((Decimal::ZERO, 0_u32, false));
            if let Some(sum) = entry.0.checked_add(q.value) {
                entry.0 = sum;
            } else {
                entry.2 = true;
            }
            entry.1 += 1;
        }
        if output.len() < 32 {
            let mut parts = vec![format!(
                "{} = {} {}",
                q.original,
                q.value.normalize(),
                q.unit
            )];
            for (label, col) in [
                ("instrument", instrument),
                ("calibrated", calibration),
                ("reference", reference),
            ] {
                if let Some(i) = col.filter(|&i| !row[i].is_empty()) {
                    parts.push(format!("{label} {}", &row[i]));
                }
            }
            if let Some((r, t)) = reference_value.zip(tolerance) {
                let reference_symbol = reference_unit.map_or(&row[u], |col| &row[col]);
                let checked = Quantity::parse(&row[r], reference_symbol)
                    .zip(q.uncertainty(&row[t], &row[u]))
                    .and_then(|(reference, tolerance)| {
                        q.against_reference(&reference, tolerance)
                            .map(|(delta, within)| (delta, within, tolerance))
                    });
                parts.push(checked.map_or_else(||"Reference comparison unavailable for these values or units".into(),|(delta,within,tolerance)|format!("reference {} {reference_symbol} · error {} {} · {} tolerance of ±{} {}",&row[r],delta.normalize(),q.unit,if within{"within"}else{"outside"},tolerance.normalize(),q.unit)));
            }
            if let Some(i) = uncertainty
                && !row[i].is_empty()
            {
                let delta = q.uncertainty(&row[i], &row[u]);
                parts.push(delta.map_or_else(
                    || "invalid stated uncertainty".into(),
                    |d| format!("uncertainty ±{} {}", d.normalize(), q.unit),
                ));
            }
            let line = row
                .position()
                .and_then(|p| u32::try_from(p.line()).ok())
                .unwrap_or(1);
            output.push(Finding {
                kind: FindingKind::Measurement,
                from: line,
                to: line,
                detail: parts.join(" · "),
            });
            quantities.push(super::MeasuredQuantity { quantity: q, line });
        }
    }
    for ((window, _, unit, series), (sum, count, overflow)) in windows.iter().take(12) {
        let mean = if *overflow {
            None
        } else {
            sum.checked_div(Decimal::from(*count))
        };
        if let Some(mean) = mean
            && let Ok(start) = OffsetDateTime::from_unix_timestamp(*window)
        {
            let label = if series.is_empty() {
                String::new()
            } else {
                format!("{series} · ")
            };
            output.push(Finding {
                kind: FindingKind::Chronology,
                from: *first,
                to: last,
                detail: format!(
                    "{label}{} · {} s window · {} reading{} · mean {} {unit}",
                    start.format(&Rfc3339).unwrap_or_default(),
                    window_seconds,
                    count,
                    if *count == 1 { "" } else { "s" },
                    mean.normalize()
                ),
            });
        }
    }
    if windows.len() > 12 {
        output.push(Finding {
            kind: FindingKind::Coverage,
            from: *first,
            to: last,
            detail: format!("Showing the earliest 12 of {} time windows.", windows.len()),
        });
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn known_reference_tolerances_keep_units_and_reject_incompatible_dimensions() {
        let data = "value,unit,reference_value,reference_unit,tolerance\n0.504,kg,500,g,0.005\n0.506,kg,500,g,0.005\n0.501,kg,500,m,0.005";
        let report =
            super::super::analyze(data, &super::super::AnalysisOptions::default()).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.from == 2 && f.detail.contains("within tolerance of ±5 g"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.from == 3 && f.detail.contains("outside tolerance of ±5 g"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.from == 4 && f.detail.contains("unavailable"))
        );
    }
}
