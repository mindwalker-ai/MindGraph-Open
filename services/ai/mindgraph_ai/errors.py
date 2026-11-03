"""Error categories of the AI service and their HTTP mapping.

MindGraph Core maps these codes onto its own ``AiError`` categories. Messages
are fixed, client-safe sentences: never prompts, dataset values, provider
response bodies, URLs or credentials.
"""

from __future__ import annotations


class AIServiceError(Exception):
    """Base class: a stable ``code`` and an HTTP ``status_code``."""

    code = "INTERNAL_ERROR"
    status_code = 500

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message


class NotConfiguredError(AIServiceError):
    code = "AI_NOT_CONFIGURED"
    status_code = 503

    def __init__(self) -> None:
        super().__init__("the AI Graph Builder is not configured")


class ProviderTimeoutError(AIServiceError):
    code = "PROVIDER_TIMEOUT"
    status_code = 504


class ProviderRateLimitedError(AIServiceError):
    code = "PROVIDER_RATE_LIMITED"
    status_code = 429


class ProviderUnavailableError(AIServiceError):
    code = "PROVIDER_UNAVAILABLE"
    status_code = 503


class ProviderRejectedError(AIServiceError):
    """Permanent provider errors: authentication, bad request, unknown model."""

    code = "PROVIDER_REJECTED"
    status_code = 502


class InvalidProviderOutputError(AIServiceError):
    """The model did not return one JSON object matching the schema."""

    code = "INVALID_PROVIDER_OUTPUT"
    status_code = 502
