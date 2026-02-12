use anyhow::Result;
use serde::Serialize;

/// Pretty-print any serializable value as JSON to stdout.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    println!("{json}");
    Ok(())
}
