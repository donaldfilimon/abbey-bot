//! Fixed owner-file credential loading after the canonical privacy transaction.
//! Parsing never invokes a shell or modifies the process environment.
use crate::{
    persist::{PersistComponentOutcome, PersistReport},
    readiness::private::PrivateDirectory,
};
use std::{collections::BTreeMap, ffi::OsString, fmt, path::Path};
const MAX_ENV_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedEnvironmentError {
    PrivacyNotCommitted,
    UnsafeFile,
    Syntax,
    RequiredConfiguration,
}
impl fmt::Display for ManagedEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("managed environment initialization failed")
    }
}
impl std::error::Error for ManagedEnvironmentError {}
pub struct ManagedEnvironment {
    values: BTreeMap<OsString, OsString>,
}
impl fmt::Debug for ManagedEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ManagedEnvironment([redacted])")
    }
}
impl ManagedEnvironment {
    pub fn load_after_privacy(
        home: &Path,
        report: &PersistReport,
    ) -> Result<Self, ManagedEnvironmentError> {
        if report.canonical_state != PersistComponentOutcome::Committed {
            return Err(ManagedEnvironmentError::PrivacyNotCommitted);
        }
        let directory = PrivateDirectory::open_existing(home, &[".config", "abbey-bot"])
            .map_err(|_| ManagedEnvironmentError::UnsafeFile)?;
        let bytes = directory
            .read_required("env", MAX_ENV_BYTES)
            .map_err(|_| ManagedEnvironmentError::UnsafeFile)?;
        Self::parse(&bytes, home)
    }
    fn parse(bytes: &[u8], home: &Path) -> Result<Self, ManagedEnvironmentError> {
        if bytes.len() > MAX_ENV_BYTES || bytes.contains(&0) {
            return Err(ManagedEnvironmentError::Syntax);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| ManagedEnvironmentError::Syntax)?;
        let mut values = BTreeMap::new();
        for line in text.lines() {
            let mut line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("export")
                && rest.starts_with(char::is_whitespace)
            {
                line = rest.trim_start();
            }
            let (key, value) = line
                .split_once('=')
                .ok_or(ManagedEnvironmentError::Syntax)?;
            if key.is_empty()
                || !key.bytes().enumerate().all(|(index, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (index > 0 && b.is_ascii_digit())
                })
            {
                return Err(ManagedEnvironmentError::Syntax);
            }
            let value = value.trim();
            let value = if value.starts_with(['\'', '"']) {
                let quote = value.as_bytes()[0];
                if value.len() < 2 || value.as_bytes().last() != Some(&quote) {
                    return Err(ManagedEnvironmentError::Syntax);
                }
                &value[1..value.len() - 1]
            } else {
                value
            };
            // Literal expansion-looking text remains literal data. Never evaluate
            // `$()`, backticks, escapes, variable references or shell operators.
            if values
                .insert(OsString::from(key), OsString::from(value))
                .is_some()
            {
                return Err(ManagedEnvironmentError::Syntax);
            }
        }
        let present = |key: &str| {
            values
                .get(std::ffi::OsStr::new(key))
                .is_some_and(|value| value.to_str().is_some_and(|text| !text.trim().is_empty()))
        };
        if !present("DISCORD_TOKEN")
            || present("ABBEY_VOICE_GUILD_ID") != present("ABBEY_VOICE_CHANNEL_ID")
            || (present("ABBEY_VOICE_GUILD_ID") && !present("ABBEY_BOT_LLM_ENDPOINT"))
        {
            return Err(ManagedEnvironmentError::RequiredConfiguration);
        }
        values.insert("HOME".into(), home.as_os_str().to_owned());
        values.insert(
            "ABBEY_DATA_DIR".into(),
            home.join(".local/share/abbey-bot").into_os_string(),
        );
        Ok(Self { values })
    }
    /// Consuming transfer for the proven single-threaded startup boundary, before
    /// constructing Tokio, subscribers, providers, or any application threads.
    /// The caller must not Debug/serialize the returned credential-bearing values.
    pub fn into_values(self) -> BTreeMap<OsString, OsString> {
        self.values
    }
}
#[cfg(test)]
mod tests;
