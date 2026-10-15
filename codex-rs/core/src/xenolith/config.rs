// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

//! Resolve Xenolith capabilities through the existing custom-model catalog seam.
//! Keeping native configuration here avoids changing upstream configuration schemas.
use crate::config::Config;
use codex_model_provider_info::WireApi;
use codex_models_manager::model_info::model_info_from_slug;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::openai_models::{
    InputModality, ModelInfo, ModelVisibility, ModelsResponse, ReasoningEffort,
    ReasoningEffortPreset,
};
use codex_xenolith_wire::backend::ReasoningOptions;
use codex_xenolith_wire::protocol::Describe;
use codex_xenolith_wire::service::{ServiceSettings, ensure_service};
use std::io;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub(super) struct GenerationSettings {
    pub max_tokens: Option<u64>,
    pub reasoning: Option<ReasoningOptions>,
}

impl GenerationSettings {
    pub fn from_env(effort: Option<&ReasoningEffort>) -> io::Result<Self> {
        Self::parse(effort, |name| {
            std::env::var(name).map(Some).or_else(|error| match error {
                std::env::VarError::NotPresent => Ok(None),
                _ => Err(invalid(format!("{name} must be UTF-8"))),
            })
        })
    }

    fn parse(
        effort: Option<&ReasoningEffort>,
        get: impl Fn(&str) -> io::Result<Option<String>>,
    ) -> io::Result<Self> {
        let integer = |name: &str, allow_zero: bool| -> io::Result<Option<u64>> {
            get(name)?
                .map(|value| {
                    let n = value
                        .parse::<u64>()
                        .map_err(|_| invalid(format!("{name} must be an integer")))?;
                    if (!allow_zero && n == 0) || n > i32::MAX as u64 {
                        return Err(invalid(format!(
                            "{name} is outside the Xenolith integer range"
                        )));
                    }
                    Ok(n)
                })
                .transpose()
        };
        let max_tokens = integer("XENOLITH_MAX_OUTPUT_TOKENS", false)?;
        let budget_tokens = integer("XENOLITH_REASONING_BUDGET", true)?;
        let history = get("XENOLITH_REASONING_HISTORY")?.unwrap_or_else(|| "discard".to_owned());
        if !matches!(history.as_str(), "discard" | "preserve_tool_calls") {
            return Err(invalid(
                "XENOLITH_REASONING_HISTORY must be discard or preserve_tool_calls",
            ));
        }
        let effort = match effort {
            None | Some(ReasoningEffort::None) => None,
            Some(ReasoningEffort::Minimal) => Some("low"),
            Some(ReasoningEffort::XHigh) => Some("high"),
            Some(value) if matches!(value.as_str(), "low" | "medium" | "high" | "max") => {
                Some(value.as_str())
            }
            Some(value) => {
                return Err(invalid(format!(
                    "Xenolith does not support reasoning effort {}",
                    value.as_str()
                )));
            }
        };
        if effort.is_none() && budget_tokens.is_some() {
            return Err(invalid(
                "XENOLITH_REASONING_BUDGET requires an enabled model_reasoning_effort",
            ));
        }
        Ok(Self {
            max_tokens,
            reasoning: effort.map(|effort| ReasoningOptions {
                effort: effort.to_owned(),
                budget_tokens,
                history,
            }),
        })
    }

    fn validate(&self, info: &Describe) -> io::Result<()> {
        if self.max_tokens.is_some_and(|limit| limit > info.max_output) {
            return Err(invalid(format!(
                "XENOLITH_MAX_OUTPUT_TOKENS exceeds describe.max_output ({})",
                info.max_output
            )));
        }
        if let Some(reasoning) = &self.reasoning {
            if !info.reasoning.efforts.contains(&reasoning.effort) {
                return Err(invalid(format!(
                    "Xenolith does not advertise reasoning effort {}",
                    reasoning.effort
                )));
            }
            if !info.reasoning.history.contains(&reasoning.history) {
                return Err(invalid(format!(
                    "Xenolith does not advertise reasoning history {}",
                    reasoning.history
                )));
            }
            if reasoning.budget_tokens.is_some() && !info.reasoning.budget_tokens {
                return Err(invalid(
                    "Xenolith does not advertise reasoning token budgets",
                ));
            }
        }
        Ok(())
    }
}

pub async fn configure(mut config: Config) -> io::Result<Config> {
    if config.model_provider.wire_api != WireApi::Xenolith {
        return Ok(config);
    }
    let generation = GenerationSettings::from_env(config.model_reasoning_effort.as_ref())?;
    let settings = ServiceSettings::from_env(config.model_provider.base_url.as_deref())?;
    let socket = settings.socket.clone();
    let requested_context = settings.context;
    let info = tokio::task::spawn_blocking(move || ensure_service(&settings))
        .await
        .map_err(io::Error::other)??;
    generation.validate(&info)?;
    if requested_context.is_some_and(|requested| requested != info.context_window) {
        config.startup_warnings.push(format!(
            "XENOLITH_CONTEXT requested {} tokens, but the running Xenolith service has {}; using the service capacity. Restart the engine to change it.",
            requested_context.unwrap_or_default(), info.context_window,
        ));
    }
    if !info.kvstore {
        return Err(invalid(
            "CodeXe requires Xenolith persistent state/cache (kvstore)",
        ));
    }
    if config
        .model
        .as_ref()
        .is_some_and(|model| model != &info.model)
    {
        return Err(invalid(format!(
            "configured model differs from Xenolith's served model {}",
            info.model
        )));
    }
    let context = config
        .model_context_window
        .unwrap_or(info.context_window as i64);
    if context <= 0 || context as u64 > info.context_window {
        return Err(invalid(format!(
            "model_context_window must be positive and no greater than Xenolith's capacity {}",
            info.context_window
        )));
    }
    if config
        .model_auto_compact_token_limit
        .is_some_and(|limit| limit <= 0 || limit > context)
    {
        return Err(invalid(
            "model_auto_compact_token_limit must fit the discovered context window",
        ));
    }
    // Reuse a user catalog's prompting/tool metadata, but never let it invent
    // capabilities or retain unrelated models in this single-model protocol.
    let template = config
        .model_catalog
        .as_ref()
        .and_then(|catalog| catalog.models.iter().find(|model| model.slug == info.model))
        .cloned();
    if config.model_catalog.is_some() && template.is_none() {
        return Err(invalid(
            "model_catalog_json must contain Xenolith's served model",
        ));
    }
    let model = discovered_model(&info, template);
    config.model_catalog = Some(ModelsResponse {
        models: vec![model],
    });
    config.model = Some(info.model);
    config.model_context_window = Some(context);
    config.model_reasoning_effort = Some(
        match generation
            .reasoning
            .as_ref()
            .map(|value| value.effort.as_str())
        {
            Some("low") => ReasoningEffort::Low,
            Some("medium") => ReasoningEffort::Medium,
            Some("high") => ReasoningEffort::High,
            Some("max") => ReasoningEffort::Max,
            _ => ReasoningEffort::None,
        },
    );
    config.model_provider.base_url = Some(format!("unix://{}", socket.display()));
    Ok(config)
}

fn discovered_model(info: &Describe, template: Option<ModelInfo>) -> ModelInfo {
    let mut model = template.unwrap_or_else(|| model_info_from_slug(&info.model));
    model.slug.clone_from(&info.model);
    model.display_name = format!("{} (Xenolith)", info.model);
    model.visibility = ModelVisibility::List;
    model.supported_in_api = true;
    model.used_fallback_model_metadata = false;
    model.default_reasoning_level = Some(ReasoningEffort::None);
    model.supported_reasoning_levels = std::iter::once(ReasoningEffort::None)
        .chain(
            info.reasoning
                .efforts
                .iter()
                .filter_map(|value| match value.as_str() {
                    "low" => Some(ReasoningEffort::Low),
                    "medium" => Some(ReasoningEffort::Medium),
                    "high" => Some(ReasoningEffort::High),
                    "max" => Some(ReasoningEffort::Max),
                    _ => None,
                }),
        )
        .map(|effort| ReasoningEffortPreset {
            description: format!("Xenolith reasoning: {}", effort.as_str()),
            effort,
        })
        .collect();
    model.context_window = Some(info.context_window as i64);
    model.max_context_window = Some(info.context_window as i64);
    model.auto_compact_token_limit = None;
    model.input_modalities = vec![InputModality::Text];
    model.supports_image_detail_original = false;
    model.supports_reasoning_summary_parameter = false;
    model.default_reasoning_summary = ReasoningSummary::None;
    model.supports_search_tool = false;
    model.use_responses_lite = false;
    model.supports_reasoning_effort_updates = false;
    model.supports_experimental_context = false;
    model.support_verbosity = false;
    model.default_verbosity = None;
    model.service_tiers.clear();
    model.additional_speed_tiers.clear();
    model.default_service_tier = None;
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(
        effort: Option<&ReasoningEffort>,
        env: &[(&str, &str)],
    ) -> io::Result<GenerationSettings> {
        GenerationSettings::parse(effort, |key| {
            Ok(env
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string()))
        })
    }

    #[test]
    fn output_defaults_to_service_and_effort_aliases_match_pi() {
        let default = settings(None, &[]).unwrap();
        assert_eq!(default.max_tokens, None);
        assert!(default.reasoning.is_none());
        for (effort, expected) in [
            (ReasoningEffort::Minimal, "low"),
            (ReasoningEffort::XHigh, "high"),
        ] {
            assert_eq!(
                settings(Some(&effort), &[])
                    .unwrap()
                    .reasoning
                    .unwrap()
                    .effort,
                expected
            );
        }
    }

    #[test]
    fn invalid_limits_and_disabled_reasoning_budget_are_rejected() {
        for env in [
            vec![("XENOLITH_MAX_OUTPUT_TOKENS", "0")],
            vec![("XENOLITH_REASONING_BUDGET", "4")],
            vec![("XENOLITH_REASONING_HISTORY", "everything")],
            vec![("XENOLITH_MAX_OUTPUT_TOKENS", "2147483648")],
        ] {
            assert!(settings(None, &env).is_err());
        }
        assert!(settings(Some(&ReasoningEffort::Ultra), &[]).is_err());
        assert_eq!(
            settings(
                Some(&ReasoningEffort::Low),
                &[("XENOLITH_REASONING_BUDGET", "0")]
            )
            .unwrap()
            .reasoning
            .unwrap()
            .budget_tokens,
            Some(0)
        );
    }
}
