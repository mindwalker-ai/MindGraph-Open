"""The OpenAI-compatible provider against an in-process mock transport."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Callable
from typing import Any

import httpx2
import pytest
from conftest import customers_suggestion

from mindgraph_ai.errors import (
    InvalidProviderOutputError,
    ProviderRateLimitedError,
    ProviderRejectedError,
    ProviderTimeoutError,
    ProviderUnavailableError,
)
from mindgraph_ai.providers import OpenAICompatibleProvider, StructuredGenerationRequest

REQUEST = StructuredGenerationRequest(
    system_prompt="system", user_prompt="user", max_output_tokens=512, request_id="job-7"
)


def completion(content: str) -> httpx2.Response:
    return httpx2.Response(200, json={"choices": [{"message": {"content": content}}]})


class Script:
    """Answers requests from a list of responses or exceptions, recording them."""

    def __init__(self, *answers: httpx2.Response | Exception | Callable[[], Any]) -> None:
        self.answers = list(answers)
        self.requests: list[httpx2.Request] = []

    def __call__(self, request: httpx2.Request) -> httpx2.Response:
        self.requests.append(request)
        answer = self.answers.pop(0)
        if isinstance(answer, Exception):
            raise answer
        return answer


def provider(script: Script, **options: Any) -> tuple[OpenAICompatibleProvider, list[float]]:
    sleeps: list[float] = []

    async def sleep(seconds: float) -> None:
        sleeps.append(seconds)

    return (
        OpenAICompatibleProvider(
            base_url="http://llm.test/v1",
            model="m",
            transport=httpx2.MockTransport(script),
            sleep=sleep,
            **options,
        ),
        sleeps,
    )


def run(p: OpenAICompatibleProvider) -> dict[str, Any]:
    return asyncio.run(p.generate_structured(REQUEST))


def test_returns_the_parsed_json_object_and_sends_a_chat_completion() -> None:
    script = Script(completion(json.dumps(customers_suggestion())))
    p, _ = provider(script, api_key="sk-secret")
    assert run(p) == customers_suggestion()
    sent = script.requests[0]
    assert sent.url == "http://llm.test/v1/chat/completions"
    assert sent.headers["authorization"] == "Bearer sk-secret"
    assert sent.headers["x-request-id"] == "job-7"
    payload = json.loads(sent.content)
    assert payload["model"] == "m"
    assert payload["temperature"] == 0
    assert payload["max_tokens"] == 512
    assert payload["response_format"] == {"type": "json_object"}
    assert [m["role"] for m in payload["messages"]] == ["system", "user"]
    assert "sk-secret" not in repr(p)


def test_the_api_key_and_json_mode_are_optional() -> None:
    script = Script(completion("{}"))
    p, _ = provider(script, json_mode=False)
    assert run(p) == {}
    sent = script.requests[0]
    assert "authorization" not in sent.headers
    assert "response_format" not in json.loads(sent.content)


@pytest.mark.parametrize(
    "content",
    [
        "Here is your result: {}",
        '```json\n{"a": 1}\n```',
        "[1, 2]",
        "{not json}",
        '{"a": 1} trailing words',
    ],
)
def test_non_json_output_is_rejected(content: str) -> None:
    p, _ = provider(Script(completion(content)))
    with pytest.raises(InvalidProviderOutputError):
        run(p)


def test_missing_message_content_is_rejected() -> None:
    p, _ = provider(Script(httpx2.Response(200, json={"choices": []})))
    with pytest.raises(InvalidProviderOutputError):
        run(p)


def test_oversized_responses_are_rejected() -> None:
    p, _ = provider(Script(completion('{"a": "' + "x" * (300 * 1024) + '"}')))
    with pytest.raises(InvalidProviderOutputError):
        run(p)


@pytest.mark.parametrize("status", [400, 401, 403, 404])
def test_permanent_errors_are_not_retried(status: int) -> None:
    script = Script(httpx2.Response(status, json={"error": "nope"}), completion("{}"))
    p, sleeps = provider(script)
    with pytest.raises(ProviderRejectedError) as error:
        run(p)
    assert len(script.requests) == 1 and sleeps == []
    assert "nope" not in error.value.message


def test_rate_limits_are_retried_with_backoff() -> None:
    script = Script(
        httpx2.Response(429, headers={"retry-after": "2"}),
        httpx2.Response(429),
        completion("{}"),
    )
    p, sleeps = provider(script)
    assert run(p) == {}
    assert len(script.requests) == 3
    assert sleeps == [2.0, 1.0]


def test_persistent_rate_limits_fail_after_the_retries() -> None:
    p, sleeps = provider(Script(*[httpx2.Response(429)] * 3))
    with pytest.raises(ProviderRateLimitedError):
        run(p)
    assert len(sleeps) == 2


@pytest.mark.parametrize("status", [500, 502, 503, 504])
def test_temporary_server_errors_are_retried(status: int) -> None:
    script = Script(httpx2.Response(status), completion("{}"))
    p, sleeps = provider(script)
    assert run(p) == {}
    assert sleeps == [0.5]


def test_server_errors_fail_after_the_retries() -> None:
    p, sleeps = provider(Script(*[httpx2.Response(503)] * 3))
    with pytest.raises(ProviderUnavailableError):
        run(p)
    assert sleeps == [0.5, 1.0]


def test_dropped_connections_are_retried() -> None:
    script = Script(httpx2.ConnectError("reset"), completion("{}"))
    p, sleeps = provider(script)
    assert run(p) == {}
    assert len(sleeps) == 1

    p, _ = provider(Script(*[httpx2.ConnectError("reset")] * 3))
    with pytest.raises(ProviderUnavailableError):
        run(p)


def test_timeouts_are_not_retried() -> None:
    script = Script(httpx2.ReadTimeout("slow"), completion("{}"))
    p, sleeps = provider(script)
    with pytest.raises(ProviderTimeoutError):
        run(p)
    assert len(script.requests) == 1 and sleeps == []


def test_retries_can_be_disabled() -> None:
    p, sleeps = provider(Script(httpx2.Response(503)), max_retries=0)
    with pytest.raises(ProviderUnavailableError):
        run(p)
    assert sleeps == []
