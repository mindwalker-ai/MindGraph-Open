"""The LLM provider interface (ADR-007).

The Graph Builder depends only on :class:`LLMProvider`; no vendor SDK or
response object leaks into it. A provider returns one parsed JSON object;
schema validation happens afterwards, in the Graph Builder.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Protocol

from mindgraph_ai.errors import InvalidProviderOutputError


@dataclass(frozen=True)
class StructuredGenerationRequest:
    """One structured-output request. Prompts are never logged."""

    system_prompt: str
    user_prompt: str
    max_output_tokens: int
    request_id: str | None = None


class LLMProvider(Protocol):
    """A model that returns one JSON object per request."""

    #: The provider kind, for example ``openai_compatible``.
    name: str
    #: The configured model name.
    model: str

    async def generate_structured(self, request: StructuredGenerationRequest) -> dict[str, Any]:
        """Returns the model's output parsed as one JSON object.

        Raises one of the ``mindgraph_ai.errors`` provider errors.
        """
        ...

    async def aclose(self) -> None:
        """Releases connections."""
        ...


def parse_strict_json(text: str) -> dict[str, Any]:
    """Parses model output that must be exactly one JSON object.

    Markdown fences, prose before or after the object and anything that is
    not a JSON object are rejected rather than repaired.
    """
    stripped = text.strip()
    if not (stripped.startswith("{") and stripped.endswith("}")):
        raise InvalidProviderOutputError("the model output is not a single JSON object")
    try:
        value = json.loads(stripped)
    except json.JSONDecodeError:
        raise InvalidProviderOutputError("the model output is not valid JSON") from None
    if not isinstance(value, dict):
        raise InvalidProviderOutputError("the model output is not a JSON object")
    return value
