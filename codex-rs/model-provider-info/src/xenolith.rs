// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

//! Built-in native defaults without HTTP authentication or model assumptions.

use crate::ModelProviderInfo;
use crate::OPENAI_PROVIDER_ID;
use crate::WireApi;

pub const XENOLITH_PROVIDER_ID: &str = "xenolith";
pub const DEFAULT_MODEL_PROVIDER_ID: &str = if cfg!(unix) {
    XENOLITH_PROVIDER_ID
} else {
    OPENAI_PROVIDER_ID
};

pub(crate) fn provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Xenolith".to_owned(),
        wire_api: WireApi::Xenolith,
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        ..Default::default()
    }
}

pub(crate) fn merge_override(configured: ModelProviderInfo) -> ModelProviderInfo {
    let mut native = provider();
    if !configured.name.trim().is_empty() {
        native.name = configured.name;
    }
    native.base_url = configured.base_url;
    native.mmproj = configured.mmproj;
    native.image_budget = configured.image_budget;
    native.request_max_retries = configured
        .request_max_retries
        .or(native.request_max_retries);
    native.stream_max_retries = configured.stream_max_retries.or(native.stream_max_retries);
    native.stream_idle_timeout_ms = configured.stream_idle_timeout_ms;
    // The reserved native identity determines transport. Deserialization cannot
    // distinguish omitted wire_api from Responses, so a socket-only override
    // must retain native defaults. HTTP credentials and capabilities stay unset.
    native
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::built_in_model_providers;
    use crate::merge_configured_model_providers;
    use std::collections::HashMap;

    #[test]
    fn builtin_xenolith_needs_no_endpoint_or_auth_configuration() {
        let providers = built_in_model_providers(None);
        let native = &providers[XENOLITH_PROVIDER_ID];
        assert_eq!(native.name, "Xenolith");
        assert_eq!(native.wire_api, WireApi::Xenolith);
        assert!(native.base_url.is_none());
        assert!(native.env_key.is_none());
        assert!(native.auth.is_none());
        assert!(!native.requires_openai_auth);
        assert!(!native.supports_websockets);
        assert!(!native.supports_standalone_web_search);
        assert_eq!(native.request_max_retries, Some(0));
        assert_eq!(native.stream_max_retries, Some(0));
    }

    #[test]
    fn native_socket_only_override_retains_native_transport() {
        let configured: ModelProviderInfo =
            toml::from_str("base_url = 'unix:///tmp/private-xenolith.sock'\nmmproj = '/models/mmproj.gguf'\nimage_budget = 140").unwrap();
        let providers = merge_configured_model_providers(
            built_in_model_providers(None),
            HashMap::from([(XENOLITH_PROVIDER_ID.to_owned(), configured)]),
        )
        .unwrap();
        let native = &providers[XENOLITH_PROVIDER_ID];
        assert_eq!(native.name, "Xenolith");
        assert_eq!(native.wire_api, WireApi::Xenolith);
        assert_eq!(
            native.base_url.as_deref(),
            Some("unix:///tmp/private-xenolith.sock")
        );
        assert!(!native.requires_openai_auth);
        assert_eq!(
            native.mmproj.as_deref(),
            Some(std::path::Path::new("/models/mmproj.gguf"))
        );
        assert_eq!(native.image_budget, Some(140));
    }

    #[test]
    fn native_overrides_keep_socket_and_timeouts_without_http_authentication() {
        let configured = ModelProviderInfo {
            name: "Local engine".to_owned(),
            base_url: Some("unix:///tmp/local-xenolith.sock".to_owned()),
            env_key: Some("UNUSED_NATIVE_API_KEY".to_owned()),
            requires_openai_auth: true,
            supports_websockets: true,
            supports_standalone_web_search: true,
            request_max_retries: Some(0),
            stream_max_retries: Some(0),
            stream_idle_timeout_ms: Some(600_000),
            ..Default::default()
        };
        let native = merge_override(configured);
        assert_eq!(native.name, "Local engine");
        assert_eq!(
            native.base_url.as_deref(),
            Some("unix:///tmp/local-xenolith.sock")
        );
        assert_eq!(native.stream_idle_timeout_ms, Some(600_000));
        assert_eq!(native.wire_api, WireApi::Xenolith);
        assert!(native.env_key.is_none());
        assert!(!native.requires_openai_auth);
        assert!(!native.supports_websockets);
        assert!(!native.supports_standalone_web_search);
    }
}
