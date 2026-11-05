"""Semantic checks of model output beyond the Pydantic schemas.

Interpretations must name only ontology types, properties of the anchored
type, existing references (selected entities and, for follow-ups, entities
of earlier questions) and depths within the limits;
anything else is rejected as invalid provider output. Answers keep only
citations of references present in the evidence: unknown ones are removed
(with a warning), never kept. MindGraph Core repeats both checks
authoritatively; these stop bad output early and keep error messages free
of model text.
"""

from __future__ import annotations

import re
from typing import Any

from mindgraph_ai.errors import InvalidProviderOutputError
from mindgraph_ai.graph_rag.schemas import (
    MAX_WARNINGS,
    Anchor,
    InterpretRequest,
    RetrievalIntent,
    SynthesizedAnswer,
    SynthesizeRequest,
)

_INLINE_CITATION = re.compile(r"\s*\[([ERP]\d+(?:\s*,\s*[ERP]\d+)*)\]")


def _reject(message: str) -> InvalidProviderOutputError:
    return InvalidProviderOutputError(f"the model's retrieval plan {message}")


def validate_intent(intent: RetrievalIntent, request: InterpretRequest) -> None:
    """Raises ``InvalidProviderOutputError`` unless the intent fits the
    ontology, the known entities and the limits."""
    properties = {
        entity_type.name: {prop.name for prop in entity_type.properties}
        for entity_type in request.ontology.entity_types
    }
    relationship_types = {r.name for r in request.ontology.relationship_types}
    known = {entity.ref for entity in request.selected_entities} | {
        entity.ref for entity in request.earlier_entities
    }

    def entity_type(name: str | None) -> None:
        if name is not None and name not in properties:
            raise _reject("names an entity type that is not in the ontology")

    def anchor(value: Anchor | None) -> None:
        if value is None:
            return
        if value.ref is not None and value.ref not in known:
            raise _reject("refers to an entity reference that does not exist")
        if value.entity_type is not None:
            entity_type(value.entity_type)
            unknown = set(value.match or {}) - properties[value.entity_type]
            if unknown:
                raise _reject("matches a property that the entity type does not have")

    for value in (intent.entity_type, intent.shared_entity_type):
        entity_type(value)
    anchor(intent.source)
    anchor(intent.target)
    if intent.relationship_type is not None and intent.relationship_type not in relationship_types:
        raise _reject("names a relationship type that is not in the ontology")

    operation = intent.operation
    if operation in ("entity_lookup", "neighbors") and intent.source is None:
        raise _reject(f"is missing the source of {operation}")
    if operation == "find_path" and (intent.source is None or intent.target is None):
        raise _reject("is missing the source or target of find_path")
    if operation == "shared_neighbor" and intent.shared_entity_type is None:
        raise _reject("is missing the shared entity type")
    depth = intent.max_depth
    if depth is not None:
        if operation == "neighbors" and depth > request.limits.max_traversal_depth:
            raise _reject("asks for a deeper neighbourhood than allowed")
        if operation == "find_path" and depth > request.limits.max_path_depth:
            raise _reject("asks for a longer path than allowed")


def evidence_refs(context: dict[str, Any]) -> set[str]:
    """The references present in an evidence context."""
    refs: set[str] = set()
    for kind in ("entities", "relationships", "paths"):
        items = context.get(kind)
        if not isinstance(items, list):
            continue
        for item in items:
            if isinstance(item, dict) and isinstance(item.get("ref"), str):
                refs.add(item["ref"])
    return refs


def check_answer(answer: SynthesizedAnswer, request: SynthesizeRequest) -> SynthesizedAnswer:
    """The answer with citations of unknown references removed (from the
    list and from the text) and a warning when something was removed. With
    no evidence at all, the answer is marked insufficient."""
    known = evidence_refs(request.context)
    removed = 0

    def inline(match: re.Match[str]) -> str:
        nonlocal removed
        refs = [ref.strip() for ref in match.group(1).split(",")]
        kept = [ref for ref in refs if ref in known]
        removed += len(refs) - len(kept)
        return f" [{', '.join(kept)}]" if kept else ""

    text = _INLINE_CITATION.sub(inline, answer.answer).strip() or answer.answer
    kept_refs: list[str] = []
    for ref in answer.evidence_refs:
        if ref in known:
            if ref not in kept_refs:
                kept_refs.append(ref)
        else:
            removed += 1
    warnings = list(answer.warnings)
    if removed:
        warnings.insert(
            0, f"Removed {removed} citation(s) that do not refer to the retrieved evidence."
        )
    return answer.model_copy(
        update={
            "answer": text,
            "evidence_refs": kept_refs,
            "insufficient_evidence": answer.insufficient_evidence or not known,
            "warnings": warnings[:MAX_WARNINGS],
        }
    )
