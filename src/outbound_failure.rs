//! Closed, content-free delivery failures. Adapters discard raw errors here.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundFailureCategory {
    Permission,
    RateLimited,
    Transport,
    Capacity,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryCertainty {
    NotSent,
    PossiblySent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OutboundFailure {
    category: OutboundFailureCategory,
    certainty: DeliveryCertainty,
    retry_after_secs: Option<u16>,
}

impl OutboundFailure {
    pub fn new(
        category: OutboundFailureCategory,
        certainty: DeliveryCertainty,
        retry_after_secs: Option<u64>,
    ) -> Self {
        Self {
            category,
            certainty,
            retry_after_secs: retry_after_secs.map(|seconds| seconds.min(300) as u16),
        }
    }

    pub fn http(status: Option<u16>, retry_after_secs: Option<u64>) -> Self {
        use DeliveryCertainty::{NotSent, PossiblySent};
        use OutboundFailureCategory::{Capacity, Internal, Permission, RateLimited, Transport};
        let (category, certainty) = match status {
            Some(401 | 403) => (Permission, NotSent),
            Some(429) => (RateLimited, NotSent),
            Some(413) => (Capacity, NotSent),
            Some(400..=499) => (Internal, NotSent),
            None | Some(500..=599) => (Transport, PossiblySent),
            Some(_) => (Internal, PossiblySent),
        };
        Self::new(category, certainty, retry_after_secs)
    }

    pub const fn category(self) -> OutboundFailureCategory {
        self.category
    }

    pub const fn certainty(self) -> DeliveryCertainty {
        self.certainty
    }

    pub const fn retry_after_secs(self) -> Option<u16> {
        self.retry_after_secs
    }
}

impl fmt::Debug for OutboundFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundFailure")
            .field("category", &self.category())
            .field("certainty", &self.certainty())
            .field("retry_after_secs", &self.retry_after_secs())
            .finish()
    }
}

impl fmt::Display for OutboundFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let category = match self.category() {
            OutboundFailureCategory::Permission => "permission",
            OutboundFailureCategory::RateLimited => "rate limit",
            OutboundFailureCategory::Transport => "transport",
            OutboundFailureCategory::Capacity => "capacity",
            OutboundFailureCategory::Internal => "internal",
        };
        let certainty = match self.certainty() {
            DeliveryCertainty::NotSent => "not sent",
            DeliveryCertainty::PossiblySent => "possibly sent",
        };
        write!(f, "outbound {category} failure ({certainty})")
    }
}

impl std::error::Error for OutboundFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_bounded() {
        for (raw, expected) in [
            (None, None),
            (Some(0), Some(0)),
            (Some(12), Some(12)),
            (Some(301), Some(300)),
            (Some(u64::MAX), Some(300)),
        ] {
            assert_eq!(
                OutboundFailure::new(
                    OutboundFailureCategory::RateLimited,
                    DeliveryCertainty::NotSent,
                    raw
                )
                .retry_after_secs(),
                expected
            );
        }
    }

    #[test]
    fn explicit_refusals_are_not_sent_and_transport_is_uncertain() {
        for (status, category, certainty) in [
            (
                Some(401),
                OutboundFailureCategory::Permission,
                DeliveryCertainty::NotSent,
            ),
            (
                Some(403),
                OutboundFailureCategory::Permission,
                DeliveryCertainty::NotSent,
            ),
            (
                Some(429),
                OutboundFailureCategory::RateLimited,
                DeliveryCertainty::NotSent,
            ),
            (
                Some(413),
                OutboundFailureCategory::Capacity,
                DeliveryCertainty::NotSent,
            ),
            (
                Some(400),
                OutboundFailureCategory::Internal,
                DeliveryCertainty::NotSent,
            ),
            (
                Some(503),
                OutboundFailureCategory::Transport,
                DeliveryCertainty::PossiblySent,
            ),
            (
                None,
                OutboundFailureCategory::Transport,
                DeliveryCertainty::PossiblySent,
            ),
        ] {
            let failure = OutboundFailure::http(status, None);
            assert_eq!(failure.category(), category);
            assert_eq!(failure.certainty(), certainty);
        }
    }

    #[test]
    fn debug_and_display_only_render_closed_values() {
        let failure = OutboundFailure::http(Some(403), Some(u64::MAX));
        assert_eq!(
            failure.to_string(),
            "outbound permission failure (not sent)"
        );
        assert_eq!(
            format!("{failure:?}"),
            "OutboundFailure { category: Permission, certainty: NotSent, retry_after_secs: Some(300) }"
        );
    }
}
