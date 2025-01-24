use alloy_primitives::{I256, U256};

pub mod ds;
pub mod hh;

pub fn format_units_int(x: &I256, decimals: &U256) -> String {
    let (sign, x) = x.into_sign_and_abs();
    format!("{sign}{}", format_units_uint(&x, decimals))
}

pub fn format_units_uint(x: &U256, decimals: &U256) -> String {
    match alloy_primitives::utils::Unit::new(decimals.saturating_to::<u8>()) {
        Some(units) => alloy_primitives::utils::ParseUnits::U256(*x).format_units(units),
        None => x.to_string(),
    }
}
