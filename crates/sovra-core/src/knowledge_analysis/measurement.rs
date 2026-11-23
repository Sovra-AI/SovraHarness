//! Comparable readings retain the user's original value, unit and precision.
//! The unit symbols below are a documented subset of UCUM, not a UCUM parser.

use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Dimension {
    Mass,
    Length,
    Time,
    Volume,
    Temperature,
    Pressure,
    Ratio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Quantity {
    pub original: String,
    pub dimension: Dimension,
    pub value: Decimal,
    pub unit: &'static str,
}

impl Quantity {
    pub fn parse(value: &str, symbol: &str) -> Option<Self> {
        let number = Decimal::from_str_exact(value)
            .ok()
            .or_else(|| Decimal::from_scientific(value).ok())?;
        let (dimension, unit, factor, offset) = conversion(symbol)?;
        let value_in_base = number.checked_mul(factor)?.checked_add(offset)?;
        Some(Self {
            original: format!("{value} {symbol}"),
            dimension,
            value: value_in_base,
            unit,
        })
    }

    pub fn compare(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (self.dimension == other.dimension).then(|| self.value.cmp(&other.value))
    }

    /// Signed error and agreement with the absolute tolerance supplied in base units.
    pub fn against_reference(
        &self,
        reference: &Self,
        tolerance: Decimal,
    ) -> Option<(Decimal, bool)> {
        if self.dimension != reference.dimension || tolerance.is_sign_negative() {
            return None;
        }
        let delta = self.value.checked_sub(reference.value)?;
        Some((delta, delta.abs() <= tolerance))
    }

    /// Uncertainty is a difference, so an affine unit contributes only its scale.
    pub fn uncertainty(&self, value: &str, symbol: &str) -> Option<Decimal> {
        let (dimension, _, scale, _) = conversion(symbol)?;
        let number = Decimal::from_str_exact(value).ok()?;
        if dimension != self.dimension || number.is_sign_negative() {
            return None;
        }
        number.checked_mul(scale)
    }
}

fn conversion(symbol: &str) -> Option<(Dimension, &'static str, Decimal, Decimal)> {
    use Dimension::{Length, Mass, Pressure, Ratio, Temperature, Time, Volume};
    let (dimension, base, mantissa, scale) = match symbol {
        "kg" => (Mass, "g", 1000, 0),
        "g" => (Mass, "g", 1, 0),
        "mg" => (Mass, "g", 1, 3),
        "ug" | "µg" | "μg" => (Mass, "g", 1, 6),
        "km" => (Length, "m", 1000, 0),
        "m" => (Length, "m", 1, 0),
        "cm" => (Length, "m", 1, 2),
        "mm" => (Length, "m", 1, 3),
        "um" | "µm" | "μm" => (Length, "m", 1, 6),
        "h" => (Time, "s", 3600, 0),
        "min" => (Time, "s", 60, 0),
        "s" => (Time, "s", 1, 0),
        "ms" => (Time, "s", 1, 3),
        "L" | "l" => (Volume, "L", 1, 0),
        "mL" | "ml" => (Volume, "L", 1, 3),
        "uL" | "µL" | "μL" => (Volume, "L", 1, 6),
        "K" | "Cel" | "°C" => (Temperature, "K", 1, 0),
        "kPa" => (Pressure, "Pa", 1000, 0),
        "Pa" => (Pressure, "Pa", 1, 0),
        "%" => (Ratio, "1", 1, 2),
        _ => return None,
    };
    let offset = if matches!(symbol, "Cel" | "°C") {
        Decimal::new(27315, 2)
    } else {
        Decimal::ZERO
    };
    Some((dimension, base, Decimal::new(mantissa, scale), offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_exact_units_and_preserves_precision_and_affine_uncertainty() {
        let mass = Quantity::parse("0.500", "kg").unwrap();
        assert_eq!(mass.original, "0.500 kg");
        assert_eq!(
            mass.compare(&Quantity::parse("500", "g").unwrap()),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(mass.compare(&Quantity::parse("500", "m").unwrap()), None);
        assert_eq!(mass.uncertainty("2", "mg"), Some(Decimal::new(2, 3)));
        let zero = Quantity::parse("0", "°C").unwrap();
        assert_eq!(zero.value, Decimal::new(27315, 2));
        assert_eq!(zero.uncertainty("0.2", "°C"), Some(Decimal::new(2, 1)));
        assert!(zero.uncertainty("-1", "K").is_none());
        assert!(Quantity::parse("nan", "g").is_none());
    }
}
