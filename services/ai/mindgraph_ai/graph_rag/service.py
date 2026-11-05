"""GraphRAG: interpretation and synthesis with the configured provider.

provider output → strict JSON → Pydantic → semantic checks → response.

The service never reads or writes graph data: MindGraph Core sends the
ontology, the selected entities or the bounded evidence context, and
validates every result again. Logs carry counts, sizes and durations only:
never the question, property values, prompts or model output.
"""

from __future__ import annotations

import json
import logging
import time
from typing import Any

from pydantic import ValidationError

from mindgraph_ai.errors import AIServiceError, InvalidProviderOutputError, NotConfiguredError
from mindgraph_ai.graph_rag.prompts import (
    INTERPRET_PROMPT_VERSION,
    INTERPRET_SYSTEM_PROMPT,
    SYNTHESIZE_PROMPT_VERSION,
    SYNTHESIZE_SYSTEM_PROMPT,
    build_interpret_prompt,
    build_synthesize_prompt,
)
from mindgraph_ai.graph_rag.schemas import (
    InterpretRequest,
    InterpretResponse,
    RetrievalIntent,
    SynthesizedAnswer,
    SynthesizeRequest,
    SynthesizeResponse,
)
from mindgraph_ai.graph_rag.validation import check_answer, evidence_refs, validate_intent
from mindgraph_ai.providers.base import LLMProvider, StructuredGenerationRequest
from mindgraph_ai.schemas import SuggestionMetadata

logger = logging.getLogger("mindgraph_ai.graph_rag")

NOT_CONFIGURED = "Ask MindGraph is not configured"
#: Re-asks after an interpretation fails validation (bounded, like M5's repair).
MAX_REPAIR_ATTEMPTS = 1


def _schema_errors(error: ValidationError) -> list[str]:
    """Where and why the schema failed, without the rejected values."""
    return [
        f"{'.'.join(str(part) for part in item['loc']) or 'intent'}: {item['msg']}"
        for item in error.errors()[:10]
    ]


def _items(context: dict[str, Any], kind: str) -> int:
    items = context.get(kind)
    return len(items) if isinstance(items, list) else 0


class GraphRagService:
    """Interprets questions and phrases answers with the configured provider."""

    def __init__(self, provider: LLMProvider | None, *, max_output_tokens: int) -> None:
        self._provider = provider
        self._max_output_tokens = max_output_tokens

    def _require(self) -> LLMProvider:
        if self._provider is None:
            raise NotConfiguredError(NOT_CONFIGURED)
        return self._provider

    def _metadata(self, provider: LLMProvider, prompt_version: str) -> SuggestionMetadata:
        return SuggestionMetadata(
            prompt_version=prompt_version, provider=provider.name, model=provider.model
        )

    async def interpret(self, request: InterpretRequest, request_id: str) -> InterpretResponse:
        provider = self._require()
        started = time.monotonic()

        def log(outcome: str, operation: str = "-") -> None:
            logger.info(
                "graphrag interpretation outcome=%s provider=%s model=%s prompt_version=%s "
                "operation=%s selected_count=%d conversation_turns=%d earlier_entity_count=%d "
                "entity_type_count=%d duration_ms=%d",
                outcome,
                provider.name,
                provider.model,
                INTERPRET_PROMPT_VERSION,
                operation,
                len(request.selected_entities),
                len(request.conversation),
                len(request.earlier_entities),
                len(request.ontology.entity_types),
                (time.monotonic() - started) * 1000,
            )

        repair: tuple[dict[str, Any], list[str]] | None = None
        attempt = 0
        while True:
            try:
                raw = await provider.generate_structured(
                    StructuredGenerationRequest(
                        system_prompt=INTERPRET_SYSTEM_PROMPT,
                        user_prompt=build_interpret_prompt(request, repair),
                        max_output_tokens=self._max_output_tokens,
                        request_id=request_id,
                    )
                )
            except AIServiceError as error:
                log(error.code)
                raise
            # Some models wrap the object: {"intent": {...}}.
            if set(raw) == {"intent"} and isinstance(raw["intent"], dict):
                raw = raw["intent"]
            try:
                intent = RetrievalIntent.model_validate(raw)
                validate_intent(intent, request)
                break
            except ValidationError as error:
                # Counts and locations only: messages could quote model output.
                logger.warning(
                    "interpretation rejected by the schema errors=%d attempt=%d",
                    error.error_count(),
                    attempt,
                )
                errors = _schema_errors(error)
            except InvalidProviderOutputError as error:
                logger.warning("interpretation rejected by validation attempt=%d", attempt)
                errors = [error.message]
            if attempt >= MAX_REPAIR_ATTEMPTS:
                log(InvalidProviderOutputError.code)
                raise InvalidProviderOutputError(
                    "the model did not return a valid retrieval intent"
                )
            attempt += 1
            repair = (raw, errors)
        log("ok" if attempt == 0 else "ok_after_repair", intent.operation)
        return InterpretResponse(
            intent=intent, metadata=self._metadata(provider, INTERPRET_PROMPT_VERSION)
        )

    async def synthesize(self, request: SynthesizeRequest, request_id: str) -> SynthesizeResponse:
        provider = self._require()
        started = time.monotonic()
        context_bytes = len(json.dumps(request.context, separators=(",", ":")))

        def log(outcome: str, answer: SynthesizedAnswer | None = None) -> None:
            logger.info(
                "graphrag synthesis outcome=%s provider=%s model=%s prompt_version=%s "
                "entity_count=%d relationship_count=%d path_count=%d context_bytes=%d "
                "complete=%s earlier_question_count=%d citation_count=%d insufficient=%s "
                "duration_ms=%d",
                outcome,
                provider.name,
                provider.model,
                SYNTHESIZE_PROMPT_VERSION,
                _items(request.context, "entities"),
                _items(request.context, "relationships"),
                _items(request.context, "paths"),
                context_bytes,
                request.retrieval.complete,
                len(request.earlier_questions),
                len(answer.evidence_refs) if answer else 0,
                answer.insufficient_evidence if answer else "-",
                (time.monotonic() - started) * 1000,
            )

        if not evidence_refs(request.context):
            # Nothing to phrase: never ask a model to answer without evidence.
            answer = SynthesizedAnswer(
                answer="There is insufficient evidence in the retrieved graph context to "
                "answer this question.",
                insufficient_evidence=True,
            )
            log("no_evidence", answer)
            return SynthesizeResponse(
                answer=answer, metadata=self._metadata(provider, SYNTHESIZE_PROMPT_VERSION)
            )
        try:
            raw = await provider.generate_structured(
                StructuredGenerationRequest(
                    system_prompt=SYNTHESIZE_SYSTEM_PROMPT,
                    user_prompt=build_synthesize_prompt(request),
                    max_output_tokens=self._max_output_tokens,
                    request_id=request_id,
                )
            )
        except AIServiceError as error:
            log(error.code)
            raise
        try:
            answer = SynthesizedAnswer.model_validate(raw)
        except ValidationError as error:
            logger.warning("answer rejected by the schema errors=%d", error.error_count())
            log(InvalidProviderOutputError.code)
            raise InvalidProviderOutputError(
                "the model output does not match the answer schema"
            ) from None
        answer = check_answer(answer, request)
        log("ok", answer)
        return SynthesizeResponse(
            answer=answer, metadata=self._metadata(provider, SYNTHESIZE_PROMPT_VERSION)
        )
