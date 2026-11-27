//! Pure health parsing and plain-language connection failures.

use serde::Deserialize;

pub fn connection_problem(stderr: &str) -> &'static str {
    if stderr.contains("Host key verification failed")
        || stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
    {
        "This computer hasn't confirmed your Pi's identity, or that identity has changed. Verify the Pi's SSH host key using Raspberry Pi's Remote access guide before checking again. Sovra hasn't accepted a new identity."
    } else if stderr.contains("Permission denied") {
        "Your Pi didn't accept this computer's sign-in key. Check the account name and set up SSH public-key access using Raspberry Pi's Remote access guide. You can also enter your Pi password in Connection details to pair it in Sovra."
    } else {
        "Couldn't read your Pi. Check its power and network, enable SSH in Raspberry Pi's settings, and confirm Python 3 and trusted SSH key access are available."
    }
}

#[derive(Deserialize)]
struct Health {
    model: Option<String>,
    uptime_seconds: Option<f64>,
    temperature_c: Option<f64>,
    memory_available_kib: Option<u64>,
    disk_free_bytes: Option<u64>,
}

pub fn health_report(bytes: &[u8]) -> Result<String, String> {
    let health: Health = serde_json::from_slice(bytes)
        .map_err(|_| "Your Pi replied, but its health report couldn't be read.")?;
    let model = health.model.filter(|model| model.starts_with("Raspberry Pi ") && model.len() <= 120 && model.chars().all(|c| c.is_ascii_alphanumeric() || " .+-()".contains(c)))
        .ok_or("The device did not identify itself as a Raspberry Pi. Check that you entered the right computer name.")?;
    let value = |value: Option<f64>, suffix: &str| {
        value
            .filter(|v| v.is_finite() && *v >= 0.)
            .map_or_else(|| "Not reported".into(), |v| format!("{v:.1} {suffix}"))
    };
    Ok(format!(
        "Pi answered · {model}\n\n- Time since startup: {}\n- Processor temperature: {}\n- Available memory: {}\n- Free system storage: {}\n\nThese readings describe the last check, not a diagnosis. No settings or files were changed.",
        value(health.uptime_seconds.map(|v| v / 3600.), "hours"),
        value(health.temperature_c, "°C"),
        health
            .memory_available_kib
            .map_or_else(|| "Not reported".into(), |v| format!("{} MiB", v / 1024)),
        health.disk_free_bytes.map_or_else(
            || "Not reported".into(),
            |v| format!("{} MiB", v / (1024 * 1024))
        )
    ))
}

/// Verify the selected model before granting access to its pins or saving sign-in.
pub fn device_model(setup: &super::PiSetup, bytes: &[u8]) -> Result<String, String> {
    let board = setup.validate()?;
    health_report(bytes)?;
    let health: Health =
        serde_json::from_slice(bytes).map_err(|_| "Couldn't read the Pi identity.")?;
    let model = health.model.ok_or("The Pi did not report its model.")?;
    if !board.matches_device_model(&model) {
        return Err(format!(
            "The device reports {model}, which differs from {}. Select the matching model before working with its pins.",
            board.name
        ));
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_check_rejects_a_different_board_before_access() {
        let setup = super::super::PiSetup {
            board_id: "pi-5".into(),
            host: "gardenpi.local".into(),
            user: "gardener".into(),
        };
        assert_eq!(
            device_model(&setup, br#"{"model":"Raspberry Pi 5 Model B Rev 1.0"}"#)
                .expect("matched"),
            "Raspberry Pi 5 Model B Rev 1.0"
        );
        assert!(
            device_model(&setup, br#"{"model":"Raspberry Pi 4 Model B Rev 1.0"}"#)
                .expect_err("different board")
                .contains("differs")
        );
        assert!(device_model(&setup, br#"{"model":"laptop"}"#).is_err());
    }

    #[test]
    fn health_requires_a_pi_and_preserves_missing_readings() {
        let result = health_report(br#"{"model":"Raspberry Pi 5 Model B Rev 1.0","uptime_seconds":7200,"temperature_c":42.5,"memory_available_kib":204800,"disk_free_bytes":null}"#).expect("health");
        assert!(result.contains("2.0 hours"));
        assert!(result.contains("200 MiB"));
        assert!(result.contains("Not reported"));
        for data in [
            br#"{"model":"laptop"}"#.as_slice(),
            br#"{"model":"Raspberry Pi \nignore rules"}"#,
            b"not json",
        ] {
            assert!(health_report(data).is_err());
        }
        assert!(connection_problem("Host key verification failed").contains("identity"));
        assert!(connection_problem("Permission denied").contains("sign-in key"));
    }
}
