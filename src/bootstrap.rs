//! Independent fixed bootstrap channel, available before the log directory opens.
use crate::{
    observability::ManagedFailure,
    readiness::{RunIdentity, valid_hex},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapPhase {
    Starting,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapCode {
    None,
    ReadinessFile,
    LogDirectory,
    LogFile,
    LogWriter,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapDocument {
    schema_version: u8,
    pid: u32,
    run_nonce: String,
    executable_sha256: String,
    phase: BootstrapPhase,
    code: BootstrapCode,
}
impl BootstrapDocument {
    pub fn new(identity: &RunIdentity, phase: BootstrapPhase, code: BootstrapCode) -> Self {
        let (pid, nonce, sha) = identity.fields();
        Self {
            schema_version: 1,
            pid,
            run_nonce: nonce.into(),
            executable_sha256: sha.into(),
            phase,
            code,
        }
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ManagedFailure> {
        if bytes.len() > 512 {
            return Err(ManagedFailure::Encode);
        }
        let doc: Self = serde_json::from_slice(bytes).map_err(|_| ManagedFailure::Encode)?;
        if doc.schema_version != 1
            || doc.pid == 0
            || doc.pid > i32::MAX as u32
            || !valid_hex(&doc.run_nonce)
            || !valid_hex(&doc.executable_sha256)
        {
            return Err(ManagedFailure::Encode);
        }
        Ok(doc)
    }
    pub fn encode(&self) -> Result<Vec<u8>, ManagedFailure> {
        let mut bytes = serde_json::to_vec(self).map_err(|_| ManagedFailure::Encode)?;
        bytes.push(b'\n');
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub(crate) fn matches(&self, identity: &RunIdentity) -> bool {
        identity.matches(self.pid, &self.run_nonce)
    }
}
