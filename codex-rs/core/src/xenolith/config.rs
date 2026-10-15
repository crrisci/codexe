// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

//! Native generation settings, validated against service capabilities by the backend.
use codex_protocol::openai_models::ReasoningEffort;
use codex_xenolith_wire::backend::ReasoningOptions;
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
