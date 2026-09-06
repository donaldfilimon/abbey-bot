//! Bounded HTTP transport for configured vision providers.
use super::*;

impl Default for HttpVisionTransport {
    fn default() -> Self {
        let client = |no_proxy| {
            let builder = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none());
            let builder = if no_proxy {
                builder.no_proxy()
            } else {
                builder
            };
            builder
                .build()
                .expect("static vision transport client configuration is valid")
        };
        Self {
            remote_client: client(false),
            // Images destined for a local VLM must never transit a process-
            // wide HTTP proxy, matching the local text and speech boundary.
            loopback_client: client(true),
        }
    }
}

impl HttpVisionTransport {
    pub(super) fn client_for(&self, raw_url: &str) -> &reqwest::Client {
        if reqwest::Url::parse(raw_url).is_ok_and(|url| crate::llm::url_is_loopback(&url)) {
            &self.loopback_client
        } else {
            &self.remote_client
        }
    }
}

impl VisionTransport for HttpVisionTransport {
    fn post(
        &self,
        request: &VisionRequest,
    ) -> impl std::future::Future<Output = Result<String, VisionError>> + Send {
        let mut builder = self
            .client_for(&request.url)
            .post(&request.url)
            .json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value);
        }
        async move {
            let response = builder.send().await.map_err(|e| {
                VisionError::classified(
                    "the vision transport failed",
                    if e.is_timeout() {
                        crate::provider::ProviderFailureKind::Timeout
                    } else {
                        crate::provider::ProviderFailureKind::TransportUnavailable
                    },
                )
            })?;
            let status = response.status();
            let rejection = crate::llm::LlmError::http(
                status,
                response.headers().get(reqwest::header::RETRY_AFTER),
            );
            if status.is_success()
                && response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER)
            {
                return Err(VisionError::classified(
                    "incompatible provider delay metadata",
                    crate::provider::ProviderFailureKind::ProtocolDrift,
                ));
            }
            if !status.is_success() {
                let _ = crate::http_body::read_capped(response, 4096).await;
                return Err(VisionError::from_llm(rejection));
            }
            let body = crate::http_body::read_capped(response, 2 * 1024 * 1024)
                .await
                .map_err(|error| VisionError::from_llm(crate::llm::LlmError::body_read(error)))?;
            String::from_utf8(body).map_err(|_| {
                VisionError::internal("the vision provider returned non-UTF-8 response bytes")
            })
        }
    }
}
