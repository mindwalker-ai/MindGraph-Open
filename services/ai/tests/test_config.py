"""Environment configuration: AI optional, invalid settings disable AI."""

from __future__ import annotations

from mindgraph_ai.config import Settings
from mindgraph_ai.main import build_provider

VALID = {
    "MINDGRAPH_LLM_PROVIDER": "openai_compatible",
    "MINDGRAPH_LLM_BASE_URL": "http://localhost:11434/v1",
    "MINDGRAPH_LLM_MODEL": "qwen",
}


def test_empty_provider_disables_ai() -> None:
    settings = Settings.from_env({})
    assert not settings.configured
    assert settings.config_error is None
    assert build_provider(settings) is None


def test_openai_compatible_settings_are_read() -> None:
    settings = Settings.from_env(
        {
            **VALID,
            "MINDGRAPH_LLM_API_KEY": "sk-secret",
            "MINDGRAPH_LLM_TIMEOUT_SECONDS": "30",
            "MINDGRAPH_LLM_MAX_OUTPUT_TOKENS": "2048",
            "MINDGRAPH_LLM_JSON_MODE": "false",
        }
    )
    assert settings.configured
    assert (settings.timeout_seconds, settings.max_output_tokens) == (30.0, 2048)
    assert settings.json_mode is False
    assert "sk-secret" not in repr(settings)
    provider = build_provider(settings)
    assert provider is not None and provider.model == "qwen"


def test_the_api_key_is_optional() -> None:
    settings = Settings.from_env(VALID)
    assert settings.configured and settings.api_key is None


def test_invalid_settings_disable_ai_with_a_reason() -> None:
    cases = [
        {**VALID, "MINDGRAPH_LLM_PROVIDER": "anthropic_sdk"},
        {**VALID, "MINDGRAPH_LLM_BASE_URL": ""},
        {**VALID, "MINDGRAPH_LLM_BASE_URL": "ftp://llm"},
        {**VALID, "MINDGRAPH_LLM_BASE_URL": "https://user:pw@llm.example/v1"},
        {**VALID, "MINDGRAPH_LLM_MODEL": ""},
        {**VALID, "MINDGRAPH_LLM_TIMEOUT_SECONDS": "0"},
        {**VALID, "MINDGRAPH_LLM_MAX_OUTPUT_TOKENS": "many"},
        {**VALID, "MINDGRAPH_LLM_JSON_MODE": "maybe"},
    ]
    for env in cases:
        settings = Settings.from_env(env)
        assert not settings.configured, env
        assert settings.config_error
        assert "pw" not in settings.config_error
