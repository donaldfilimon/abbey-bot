//! Fixed recovery advice. Neither errors nor operator-supplied strings enter this boundary.
use crate::persist::{
    PersistComponentOutcome, PersistErrorCategory, PersistOverall, PersistReport,
};
use crate::provider::ProviderFailureKind;

pub fn persistence_guidance(report: &PersistReport) -> &'static str {
    // A rename followed by failed directory sync may already be visible. Never
    // describe that state as a rollback or assert that nothing was written.
    if report.canonical_state
        == PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory)
    {
        return "Canonical state may already be visible, but directory durability was not confirmed. Ask the host operator to inspect persistence before retrying; the component results above remain authoritative.";
    }
    match report.overall {
        PersistOverall::MemoryOnly => {
            "This instance has no durable state directory configured. State remains in memory; ask the host operator to configure persistence if it must survive a restart."
        }
        PersistOverall::Complete => {
            "Canonical state and its WDBX projection were both saved. This confirms this persistence attempt only."
        }
        PersistOverall::Partial => {
            "Canonical state was saved, but its WDBX projection was not confirmed durable. Ask the host operator to inspect the projection failure before retrying."
        }
        PersistOverall::Failed => {
            "Persistence did not complete. Keep the component results above when asking the host operator to investigate; a failure does not prove that earlier file changes were undone."
        }
    }
}

pub fn provider_guidance(kind: ProviderFailureKind, manager: bool) -> &'static str {
    use ProviderFailureKind as Failure;
    match kind {
        Failure::Success => "The provider request completed.",
        Failure::TransportUnavailable | Failure::Timeout | Failure::Http5xx => {
            "The provider is temporarily unavailable. Try again shortly; if it continues, ask a server manager to contact the host operator."
        }
        Failure::RateLimited => "The provider has limited requests. Wait before trying again.",
        Failure::Busy => "The provider is busy. Try again after the current requests finish.",
        Failure::Cancelled => "The request was cancelled. Start a new request when you are ready.",
        Failure::InvalidRequest => {
            "The provider could not accept this request. Check the input and try again."
        }
        Failure::Authentication | Failure::Authorization if manager => {
            "The provider rejected access. Ask the host operator to check the configured credentials and permissions, then requalify the provider."
        }
        Failure::Configuration if manager => {
            "The provider configuration needs attention. Ask the host operator to check its configuration and run the provider self-test before retrying."
        }
        Failure::ExecutableIdentity | Failure::ModelIdentity | Failure::SandboxIdentity
            if manager =>
        {
            "The provider identity no longer matches its qualification. Ask the host operator to verify the executable, model and isolation settings, then requalify the provider."
        }
        Failure::ToolSchema | Failure::ResponseSchema | Failure::ProtocolDrift if manager => {
            "The provider failed its response contract. Ask the host operator to inspect the fixed failure category and requalify the provider before retrying."
        }
        Failure::Authentication
        | Failure::Authorization
        | Failure::Configuration
        | Failure::ExecutableIdentity
        | Failure::ModelIdentity
        | Failure::SandboxIdentity
        | Failure::ToolSchema
        | Failure::ResponseSchema
        | Failure::ProtocolDrift => {
            "This provider needs attention before it can answer. Ask a server manager to contact the host operator."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_advice_preserves_partial_and_uncertain_commit_truth() {
        use PersistComponentOutcome as Outcome;
        use PersistErrorCategory as Error;
        assert!(persistence_guidance(&PersistReport::memory_only()).contains("in memory"));
        assert!(
            persistence_guidance(&PersistReport::from_components(
                Outcome::Committed,
                Outcome::Committed
            ))
            .contains("both saved")
        );
        for error in [
            Error::UnsafeFileType,
            Error::CreateDirectory,
            Error::SnapshotEncode,
            Error::CreateTemporary,
            Error::WriteTemporary,
            Error::SyncTemporary,
            Error::PublishRename,
            Error::SyncDirectory,
            Error::ProjectionEncode,
        ] {
            let partial =
                PersistReport::from_components(Outcome::Committed, Outcome::Failed(error));
            assert!(persistence_guidance(&partial).contains("projection"));
            assert!(!persistence_guidance(&partial).contains("both saved"));
            let failed = PersistReport::from_components(
                Outcome::Failed(error),
                Outcome::SkippedCanonicalFailure,
            );
            assert!(!persistence_guidance(&failed).contains("both saved"));
            if error == Error::SyncDirectory {
                assert!(persistence_guidance(&failed).contains("may already be visible"));
            }
        }
    }

    #[test]
    fn members_receive_no_host_log_or_credential_instructions() {
        use ProviderFailureKind as Failure;
        for kind in [
            Failure::Success,
            Failure::TransportUnavailable,
            Failure::Timeout,
            Failure::Http5xx,
            Failure::RateLimited,
            Failure::Authentication,
            Failure::Authorization,
            Failure::Configuration,
            Failure::ExecutableIdentity,
            Failure::ModelIdentity,
            Failure::SandboxIdentity,
            Failure::ToolSchema,
            Failure::ResponseSchema,
            Failure::ProtocolDrift,
            Failure::Cancelled,
            Failure::InvalidRequest,
            Failure::Busy,
        ] {
            let member = provider_guidance(kind, false);
            assert!(!member.is_empty());
            assert!(!member.contains("logs"));
            assert!(!member.contains("credentials"));
            assert!(!member.contains("/Users/"));
            assert!(!member.contains("DISCORD_TOKEN"));
        }
        assert!(provider_guidance(Failure::Authentication, true).contains("credentials"));
        assert!(provider_guidance(Failure::ResponseSchema, true).contains("requalify"));
    }
}
