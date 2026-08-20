//! Provider capabilities and deterministic routing.
//!
//! A configured endpoint is not evidence that it can satisfy a request.  The
//! runtime therefore routes against a semantic capability set, and a provider
//! must satisfy every bit required by the current operation.  Foundation
//! Models is considered only when the operator explicitly enables fallback;
//! its server and schema-constrained CLI are separate routes because `fm
//! serve` must never be advertised as tool-capable.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::llm::Backend;

/// Independently qualified provider behavior.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub tools: bool,
    pub vision: bool,
    pub ocr: bool,
}

impl ProviderCapabilities {
    /// Whether this capability set is a superset of the request.
    #[must_use]
    pub const fn satisfies(self, required: Self) -> bool {
        (!required.text || self.text)
            && (!required.streaming || self.streaming)
            && (!required.structured_output || self.structured_output)
            && (!required.tools || self.tools)
            && (!required.vision || self.vision)
            && (!required.ocr || self.ocr)
    }

    /// Capabilities of Abbey's existing text backend contract. Vision remains
    /// separately qualified, and the Anthropic path is intentionally not
    /// described as streamed by this crate.
    #[must_use]
    pub const fn primary(backend: &Backend, tools: bool) -> Self {
        Self {
            text: true,
            streaming: matches!(backend, Backend::OpenAiCompatible { .. }),
            structured_output: tools,
            tools,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: false,
            tools: false,
            vision: false,
            ocr: false,
        }
    }

    #[must_use]
    pub const fn text_with_tools() -> Self {
        Self {
            text: true,
            streaming: false,
            structured_output: true,
            tools: true,
            vision: false,
            ocr: false,
        }
    }
}

/// A concrete transport path. The FM server and CLI are deliberately distinct
/// so qualification of one can never inflate the other's privileges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRoute {
    Primary,
    FoundationModelsServer,
    FoundationModelsCli,
}

/// Immutable capability evidence plus per-provider runtime feature state.
pub struct ProviderRouter {
    primary: Option<ProviderCapabilities>,
    fm_server: Option<ProviderCapabilities>,
    fm_cli: Option<ProviderCapabilities>,
    fm_fallback: bool,
    primary_tools_enabled: AtomicBool,
    fm_tools_enabled: AtomicBool,
}

impl ProviderRouter {
    #[must_use]
    pub fn new(
        primary_backend: Option<&Backend>,
        primary_tools_enabled: bool,
        fm_capabilities: Option<ProviderCapabilities>,
        fm_fallback: bool,
    ) -> Self {
        let primary = primary_backend
            .map(|backend| ProviderCapabilities::primary(backend, primary_tools_enabled));
        // The OpenAI-compatible server path is never tool-capable. Structured
        // output and Abbey tool selection belong exclusively to `fm respond`.
        let fm_server = fm_capabilities.map(|caps| ProviderCapabilities {
            structured_output: false,
            tools: false,
            ..caps
        });
        // The CLI adapter is not a streamed/image transport. It is selected
        // only for typed final answers and typed Abbey tool requests.
        let fm_cli = fm_capabilities.map(|caps| ProviderCapabilities {
            streaming: false,
            vision: false,
            ocr: false,
            ..caps
        });
        Self {
            primary,
            fm_server,
            fm_cli,
            fm_fallback,
            primary_tools_enabled: AtomicBool::new(primary_tools_enabled),
            fm_tools_enabled: AtomicBool::new(
                fm_capabilities.is_some_and(|capabilities| capabilities.tools),
            ),
        }
    }

    /// Ordered eligible routes. FM is absent unless explicit fallback is on,
    /// and only routes satisfying every requested capability are returned.
    #[must_use]
    pub fn candidates(&self, required: ProviderCapabilities) -> Vec<ProviderRoute> {
        let mut routes = Vec::with_capacity(2);
        if self
            .effective_capabilities(ProviderRoute::Primary)
            .is_some_and(|caps| caps.satisfies(required))
        {
            routes.push(ProviderRoute::Primary);
        }
        if !self.fm_fallback {
            return routes;
        }
        let fm_route = if required.tools || required.structured_output {
            ProviderRoute::FoundationModelsCli
        } else {
            ProviderRoute::FoundationModelsServer
        };
        if self
            .effective_capabilities(fm_route)
            .is_some_and(|caps| caps.satisfies(required))
        {
            routes.push(fm_route);
        }
        routes
    }

    /// The current capabilities after a provider-specific runtime rejection.
    #[must_use]
    pub fn effective_capabilities(&self, route: ProviderRoute) -> Option<ProviderCapabilities> {
        let mut caps = match route {
            ProviderRoute::Primary => self.primary?,
            ProviderRoute::FoundationModelsServer => self.fm_server?,
            ProviderRoute::FoundationModelsCli => self.fm_cli?,
        };
        let tools_enabled = match route {
            ProviderRoute::Primary => self.primary_tools_enabled.load(Ordering::Relaxed),
            ProviderRoute::FoundationModelsCli => self.fm_tools_enabled.load(Ordering::Relaxed),
            ProviderRoute::FoundationModelsServer => false,
        };
        if !tools_enabled {
            caps.tools = false;
            if matches!(route, ProviderRoute::Primary) {
                caps.structured_output = false;
            }
        }
        Some(caps)
    }

    /// Disable only the provider that rejected Abbey's tool contract.
    pub fn disable_tools(&self, route: ProviderRoute) {
        match route {
            ProviderRoute::Primary => self.primary_tools_enabled.store(false, Ordering::Relaxed),
            ProviderRoute::FoundationModelsCli => {
                self.fm_tools_enabled.store(false, Ordering::Relaxed);
            }
            ProviderRoute::FoundationModelsServer => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> Backend {
        Backend::OpenAiCompatible {
            endpoint: "http://127.0.0.1:8282".into(),
            model: "gemma".into(),
        }
    }

    fn qualified_fm() -> ProviderCapabilities {
        ProviderCapabilities {
            text: true,
            streaming: true,
            structured_output: true,
            tools: true,
            vision: true,
            ocr: false,
        }
    }

    #[test]
    fn capability_selection_requires_the_full_requested_set() {
        let router = ProviderRouter::new(Some(&local()), true, Some(qualified_fm()), true);
        assert_eq!(
            router.candidates(ProviderCapabilities::text()),
            [
                ProviderRoute::Primary,
                ProviderRoute::FoundationModelsServer
            ]
        );
        assert_eq!(
            router.candidates(ProviderCapabilities::text_with_tools()),
            [ProviderRoute::Primary, ProviderRoute::FoundationModelsCli]
        );
        let ocr = ProviderCapabilities {
            text: true,
            ocr: true,
            ..ProviderCapabilities::default()
        };
        assert!(router.candidates(ocr).is_empty());
    }

    #[test]
    fn fm_is_never_an_implicit_fallback() {
        let router = ProviderRouter::new(None, true, Some(qualified_fm()), false);
        assert!(router.candidates(ProviderCapabilities::text()).is_empty());
        let router = ProviderRouter::new(None, true, Some(qualified_fm()), true);
        assert_eq!(
            router.candidates(ProviderCapabilities::text()),
            [ProviderRoute::FoundationModelsServer]
        );
    }

    #[test]
    fn a_rejection_disables_only_that_providers_tools() {
        let router = ProviderRouter::new(Some(&local()), true, Some(qualified_fm()), true);
        router.disable_tools(ProviderRoute::Primary);
        assert_eq!(
            router.candidates(ProviderCapabilities::text_with_tools()),
            [ProviderRoute::FoundationModelsCli]
        );
        assert!(
            router
                .effective_capabilities(ProviderRoute::FoundationModelsServer)
                .is_some_and(|caps| !caps.tools),
            "fm serve can never inherit CLI tool qualification"
        );
    }
}
