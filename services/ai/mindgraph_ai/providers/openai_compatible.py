"""A generic OpenAI-compatible chat-completions provider.

Works with hosted providers and with local gateways that speak the same API
(vLLM, Ollama's ``/v1`` endpoint, llama.cpp server, ...):
``POST {base_url}/chat/completions``.

- One ``httpx2.AsyncClient`` per process, with a timeout per attempt.
- ``Authorization: Bearer`` only when an API key is configured.
- Bounded retries (default 2) with small exponential backoff, only for
  429, temporary 5xx and dropped connections. 400/401/403/404 are permanent.
  Timeouts are not retried: they are already the slow path.
- The response is capped at 256 KiB and must contain exactly one JSON object.
  ``response_format: json_object`` is requested when JSON mode is enabled,
  but correctness never depends on it: the output is always parsed strictly
  and validated by the Graph Builder.
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from typing import Any

import httpx2

from mindgraph_ai.errors import (
    InvalidProviderOutputError,
    ProviderRateLimitedError,
    ProviderRejectedError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from mindgraph_ai.providers.base import StructuredGenerationRequest, parse_strict_json

logger = logging.getLogger("mindgraph_ai.providers.openai_compatible")

MAX_RESPONSE_BYTES = 256 * 1024
RETRYABLE_STATUSES = frozenset({429, 500, 502, 503, 504})
MAX_RETRY_AFTER_SECONDS = 5.0


class OpenAICompatibleProvider:
    """``LLMProvider`` for any OpenAI-compatible endpoint."""

    name = "openai_compatible"

    def __init__(
        self,
        *,
        base_url: str,
        model: str,
        api_key: str | None = None,
        timeout_seconds: float = 60.0,
        max_retries: int = 2,
        json_mode: bool = True,
        backoff_seconds: float = 0.5,
        transport: httpx2.AsyncBaseTransport | None = None,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
    ) -> None:
        self.model = model
        self._api_key = api_key
        self._max_retries = max_retries
        self._json_mode = json_mode
        self._backoff = backoff_seconds
        self._sleep = sleep
        self._client = httpx2.AsyncClient(
            base_url=base_url.rstrip("/") + "/",
            timeout=timeout_seconds,
            transport=transport,
        )

    def __repr__(self) -> str:  # never shows the key or the URL
        return f"OpenAICompatibleProvider(model={self.model!r})"

    async def aclose(self) -> None:
        await self._client.aclose()

    def _headers(self, request: StructuredGenerationRequest) -> dict[str, str]:
        headers = {"Accept": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        if request.request_id:
            headers["x-request-id"] = request.request_id
        return headers

    def _payload(self, request: StructuredGenerationRequest) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "model": self.model,
            "messages": [
                {"role": "system", "content": request.system_prompt},
                {"role": "user", "content": request.user_prompt},
            ],
            "temperature": 0,
            "max_tokens": request.max_output_tokens,
        }
        if self._json_mode:
            payload["response_format"] = {"type": "json_object"}
        return payload

    def _delay(self, attempt: int, response: httpx2.Response | None) -> float:
        delay = self._backoff * (2**attempt)
        if response is not None:
            try:
                retry_after = float(response.headers.get("retry-after", ""))
            except ValueError:
                retry_after = 0.0
            delay = max(delay, min(retry_after, MAX_RETRY_AFTER_SECONDS))
        return delay

    async def generate_structured(self, request: StructuredGenerationRequest) -> dict[str, Any]:
        headers = self._headers(request)
        payload = self._payload(request)
        attempt = 0
        while True:
            try:
                response = await self._client.post(
                    "chat/completions", json=payload, headers=headers
                )
            except httpx2.TimeoutException:
                raise ProviderTimeoutError("the LLM provider did not answer in time") from None
            except httpx2.TransportError:
                if attempt < self._max_retries:
                    await self._retry(attempt, None, "connection error")
                    attempt += 1
                    continue
                raise ProviderUnavailableError("the LLM provider cannot be reached") from None

            status = response.status_code
            if status in RETRYABLE_STATUSES:
                if attempt < self._max_retries:
                    await self._retry(attempt, response, f"HTTP {status}")
                    attempt += 1
                    continue
                if status == 429:
                    raise ProviderRateLimitedError("the LLM provider is rate limiting requests")
                raise ProviderUnavailableError(f"the LLM provider failed (HTTP {status})")
            if 400 <= status < 500:
                raise ProviderRejectedError(
                    f"the LLM provider rejected the request (HTTP {status})"
                )
            if status != 200:
                raise ProviderUnavailableError(f"the LLM provider failed (HTTP {status})")
            if attempt:
                logger.info("provider call succeeded retry_count=%d", attempt)
            return self._content(response)

    async def _retry(self, attempt: int, response: httpx2.Response | None, reason: str) -> None:
        delay = self._delay(attempt, response)
        logger.warning(
            "provider call failed, retrying reason=%s retry=%d delay_s=%.2f",
            reason,
            attempt + 1,
            delay,
        )
        await self._sleep(delay)

    @staticmethod
    def _content(response: httpx2.Response) -> dict[str, Any]:
        if len(response.content) > MAX_RESPONSE_BYTES:
            raise InvalidProviderOutputError("the LLM provider response is too large")
        try:
            body = response.json()
            content = body["choices"][0]["message"]["content"]
        except (ValueError, KeyError, IndexError, TypeError):
            raise InvalidProviderOutputError(
                "the LLM provider response has no message content"
            ) from None
        if not isinstance(content, str):
            raise InvalidProviderOutputError("the LLM provider response has no message content")
        return parse_strict_json(content)
