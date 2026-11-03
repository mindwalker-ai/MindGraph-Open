"""Runtime configuration from environment variables (ADR-013).

AI is optional. With ``MINDGRAPH_LLM_PROVIDER`` empty the service starts,
``/health`` is healthy, ``/ready`` reports ``not_configured``, and MindGraph
Core keeps every manual flow working.

The API key is never logged, returned or included in ``repr``.
"""

from __future__ import annotations

import logging
import os
from collections.abc import Mapping
from dataclasses import dataclass, field
from urllib.parse import urlsplit

logger = logging.getLogger("mindgraph_ai.config")

OPENAI_COMPATIBLE = "openai_compatible"
SUPPORTED_PROVIDERS = (OPENAI_COMPATIBLE,)

DEFAULT_TIMEOUT_SECONDS = 60.0
DEFAULT_MAX_OUTPUT_TOKENS = 4096
DEFAULT_MAX_RETRIES = 2


class ConfigError(ValueError):
    """An invalid setting. The message never contains secrets."""


@dataclass(frozen=True)
class Settings:
    """AI service settings. ``provider is None`` means AI is disabled."""

    provider: str | None = None
    base_url: str | None = None
    model: str | None = None
    api_key: str | None = field(default=None, repr=False)
    timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS
    max_output_tokens: int = DEFAULT_MAX_OUTPUT_TOKENS
    max_retries: int = DEFAULT_MAX_RETRIES
    json_mode: bool = True
    #: Why AI is disabled although a provider was named (invalid settings).
    config_error: str | None = None

    @property
    def configured(self) -> bool:
        """Whether a usable provider is configured."""
        return self.provider is not None and self.config_error is None

    @classmethod
    def from_env(cls, env: Mapping[str, str] | None = None) -> Settings:
        """Reads the settings. Invalid settings disable AI with a logged reason
        instead of crashing the service."""
        env = os.environ if env is None else env
        try:
            return cls._parse(env)
        except ConfigError as error:
            logger.error("AI Graph Builder disabled: %s", error)
            return cls(provider=None, config_error=str(error))

    @classmethod
    def _parse(cls, env: Mapping[str, str]) -> Settings:
        def text(name: str) -> str | None:
            value = env.get(name, "").strip()
            return value or None

        provider = text("MINDGRAPH_LLM_PROVIDER")
        if provider is None:
            return cls()
        if provider not in SUPPORTED_PROVIDERS:
            raise ConfigError(
                f"MINDGRAPH_LLM_PROVIDER must be one of {', '.join(SUPPORTED_PROVIDERS)} or empty"
            )
        base_url = text("MINDGRAPH_LLM_BASE_URL")
        if base_url is None:
            raise ConfigError("MINDGRAPH_LLM_BASE_URL is required when a provider is set")
        parts = urlsplit(base_url)
        if parts.scheme not in ("http", "https") or not parts.hostname:
            raise ConfigError("MINDGRAPH_LLM_BASE_URL must be an http:// or https:// URL")
        if parts.username or parts.password:
            raise ConfigError(
                "MINDGRAPH_LLM_BASE_URL must not contain credentials; use MINDGRAPH_LLM_API_KEY"
            )
        model = text("MINDGRAPH_LLM_MODEL")
        if model is None:
            raise ConfigError("MINDGRAPH_LLM_MODEL is required when a provider is set")
        return cls(
            provider=provider,
            base_url=base_url,
            model=model,
            api_key=text("MINDGRAPH_LLM_API_KEY"),
            timeout_seconds=_number(
                env, "MINDGRAPH_LLM_TIMEOUT_SECONDS", DEFAULT_TIMEOUT_SECONDS, 1, 600
            ),
            max_output_tokens=int(
                _number(
                    env, "MINDGRAPH_LLM_MAX_OUTPUT_TOKENS", DEFAULT_MAX_OUTPUT_TOKENS, 256, 32768
                )
            ),
            max_retries=int(_number(env, "MINDGRAPH_LLM_MAX_RETRIES", DEFAULT_MAX_RETRIES, 0, 2)),
            json_mode=_flag(env, "MINDGRAPH_LLM_JSON_MODE", default=True),
        )


def _number(env: Mapping[str, str], name: str, default: float, low: float, high: float) -> float:
    raw = env.get(name, "").strip()
    if not raw:
        return default
    try:
        value = float(raw)
    except ValueError:
        raise ConfigError(f"{name} must be a number from {low:g} to {high:g}") from None
    if not low <= value <= high:
        raise ConfigError(f"{name} must be a number from {low:g} to {high:g}")
    return value


def _flag(env: Mapping[str, str], name: str, *, default: bool) -> bool:
    raw = env.get(name, "").strip().lower()
    if not raw:
        return default
    if raw in ("1", "true", "yes", "on"):
        return True
    if raw in ("0", "false", "no", "off"):
        return False
    raise ConfigError(f"{name} must be true or false")
