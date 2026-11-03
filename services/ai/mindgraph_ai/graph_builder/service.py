"""The Graph Builder: provider output → strict JSON → Pydantic → response.

The service never writes anything: it returns a suggestion that MindGraph
Core validates again (authoritatively) before a user reviews it.
"""

from __future__ import annotations

import logging
import time

from pydantic import ValidationError

from mindgraph_ai.errors import AIServiceError, InvalidProviderOutputError, NotConfiguredError
from mindgraph_ai.graph_builder.prompts import PROMPT_VERSION, SYSTEM_PROMPT, build_user_prompt
from mindgraph_ai.providers.base import LLMProvider, StructuredGenerationRequest
from mindgraph_ai.schemas import (
    GraphBuilderSuggestion,
    SuggestionMetadata,
    SuggestRequest,
    SuggestResponse,
)

logger = logging.getLogger("mindgraph_ai.graph_builder")

#: Largest accepted validated suggestion, serialised.
MAX_SUGGESTION_BYTES = 256 * 1024


class GraphBuilderService:
    """Asks the configured provider for one suggestion and validates its shape."""

    def __init__(self, provider: LLMProvider | None, *, max_output_tokens: int) -> None:
        self._provider = provider
        self._max_output_tokens = max_output_tokens

    @property
    def provider(self) -> LLMProvider | None:
        return self._provider

    async def suggest(self, request: SuggestRequest, request_id: str) -> SuggestResponse:
        provider = self._provider
        if provider is None:
            raise NotConfiguredError()
        started = time.monotonic()
        generation = StructuredGenerationRequest(
            system_prompt=SYSTEM_PROMPT,
            user_prompt=build_user_prompt(request),
            max_output_tokens=self._max_output_tokens,
            request_id=request_id,
        )

        def log(outcome: str) -> None:
            logger.info(
                "graph builder suggestion outcome=%s provider=%s model=%s prompt_version=%s "
                "field_count=%d record_count=%d repair=%s duration_ms=%d",
                outcome,
                provider.name,
                provider.model,
                PROMPT_VERSION,
                len(request.dataset_profile.fields),
                request.dataset_profile.record_count,
                request.repair is not None,
                (time.monotonic() - started) * 1000,
            )

        try:
            raw = await provider.generate_structured(generation)
        except AIServiceError as error:
            log(error.code)
            raise
        try:
            suggestion = GraphBuilderSuggestion.model_validate(raw)
        except ValidationError as error:
            # Only the count: messages could quote model output.
            logger.warning("model output rejected by the schema errors=%d", error.error_count())
            log(InvalidProviderOutputError.code)
            raise InvalidProviderOutputError(
                "the model output does not match the suggestion schema"
            ) from None
        if len(suggestion.model_dump_json()) > MAX_SUGGESTION_BYTES:
            log(InvalidProviderOutputError.code)
            raise InvalidProviderOutputError("the model output is too large")
        log("ok")
        return SuggestResponse(
            suggestion=suggestion,
            metadata=SuggestionMetadata(
                prompt_version=PROMPT_VERSION, provider=provider.name, model=provider.model
            ),
        )
