//! Token-free, offline FM identity inspection before application startup.
//!
//! Reads executable bytes and the OS build using the production identity seam.
//! It never calls FM inference, loads credentials, or constructs a runtime.

use std::path::Path;

pub(crate) fn render(cli: &Path) -> Result<String, String> {
    let cli = cli
        .to_str()
        .ok_or_else(|| "FM CLI path must be UTF-8".to_string())?;
    let config = crate::provider::FmConfig::from_values(
        Some("system".into()),
        None,
        Some(cli.into()),
        None,
        None,
    )?
    .ok_or_else(|| "FM identity configuration is missing".to_string())?;
    let identity = crate::provider::fm_manifest_identity(&config)?;
    serde_json::to_string(&identity).map_err(|_| "FM identity serialization failed".into())
}

pub(crate) fn run(cli: &Path) -> Result<(), crate::Error> {
    let identity = render(cli).map_err(crate::runtime::StartupError)?;
    println!("{identity}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rejects_relative_cli_before_os_or_provider_access() {
        assert!(render(Path::new("relative-fm")).is_err());
    }
}
