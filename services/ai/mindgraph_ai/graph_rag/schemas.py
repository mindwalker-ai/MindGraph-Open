"""GraphRAG request and response schemas (Pydantic v2).

The retrieval intent mirrors MindGraph Core's ``RetrievalIntent``: four
bounded operations plus ``unsupported``, anchors that name a known entity
(``S1`` selected, ``H1`` from an earlier question of the conversation), an
entity id or an exact property match, and nothing executable. Unknown
fields are rejected everywhere. MindGraph Core validates every intent again
against the ontology and the limits (authoritatively) and checks every
citation against the evidence it retrieved.
"""

from __future__ import annotations

from typing import Annotated, Any, Literal

from pydantic import Field, StringConstraints, field_validator, model_validator

from mindgraph_ai.schemas import (
    ActiveOntology,
    MachineName,
    ShortText,
    Strict,
    SuggestionMetadata,
)

#: The same bounds as MindGraph Core.
MAX_QUESTION_CHARS = 2000
MAX_SELECTED_ENTITIES = 5
MAX_MATCH_PROPERTIES = 10
MAX_ANSWER_CHARS = 4000
MAX_WARNINGS = 10
MAX_DEPTH = 6
MAX_REASON_CHARS = 300
MAX_CONVERSATION_TURNS = 3
MAX_TURN_ENTITIES = 10
MAX_EARLIER_ENTITIES = 10

Question = Annotated[str, StringConstraints(min_length=1, max_length=MAX_QUESTION_CHARS)]
#: A selected entity: ``S1`` is the first entity of the question's scope.
SelectedRef = Annotated[str, StringConstraints(pattern=r"^S[1-9]$")]
#: An entity of an earlier question (follow-ups): ``H1`` … ``H10``.
EarlierRef = Annotated[str, StringConstraints(pattern=r"^H(?:10|[1-9])$")]
#: Either kind of known entity.
KnownRef = Annotated[str, StringConstraints(pattern=r"^(?:S[1-9]|H(?:10|[1-9]))$")]
#: A MindGraph entity id (a UUID); MindGraph Core checks the version.
EntityIdText = Annotated[
    str,
    StringConstraints(
        pattern=r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    ),
]
MatchValue = str | int | float | bool
Operation = Literal["entity_lookup", "neighbors", "find_path", "shared_neighbor", "unsupported"]
Direction = Literal["outgoing", "incoming", "both"]


class Anchor(Strict):
    """Exactly one of: a known entity (selected or from an earlier
    question), an entity id, or a type with an exact property match."""

    ref: KnownRef | None = None
    entity_id: EntityIdText | None = None
    entity_type: MachineName | None = None
    match: dict[MachineName, MatchValue] | None = Field(
        default=None, max_length=MAX_MATCH_PROPERTIES
    )

    @model_validator(mode="after")
    def _one_form(self) -> Anchor:
        forms = sum(value is not None for value in (self.ref, self.entity_id, self.entity_type))
        if forms != 1:
            raise ValueError("an anchor names exactly one of ref, entity_id or entity_type")
        if self.match is not None and self.entity_type is None:
            raise ValueError("match needs entity_type")
        return self


class RetrievalIntent(Strict):
    """A structured interpretation of a question: never a query language."""

    operation: Operation
    source: Anchor | None = None
    target: Anchor | None = None
    entity_type: MachineName | None = None
    shared_entity_type: MachineName | None = None
    relationship_type: MachineName | None = None
    #: ``None`` means ``both``.
    direction: Direction | None = None
    max_depth: int | None = Field(default=None, ge=1, le=MAX_DEPTH)
    reason: ShortText | None = None

    @field_validator("reason", mode="before")
    @classmethod
    def _bounded_reason(cls, value: object) -> object:
        """A long explanation is shortened, not a reason to reject the plan."""
        if isinstance(value, str) and len(value) > MAX_REASON_CHARS:
            return value[: MAX_REASON_CHARS - 1] + "…"
        return value


class SelectedEntity(Strict):
    ref: SelectedRef
    entity_type: MachineName
    properties: dict[str, Any] = Field(default_factory=dict, max_length=50)


class EarlierEntity(Strict):
    """An entity cited by an earlier question of the conversation."""

    ref: EarlierRef
    entity_type: MachineName
    properties: dict[str, Any] = Field(default_factory=dict, max_length=50)


class ConversationTurn(Strict):
    """An earlier question and the references of the entities it was about."""

    question: Question
    entities: list[KnownRef] = Field(default_factory=list, max_length=MAX_TURN_ENTITIES)


class InterpretLimits(Strict):
    max_traversal_depth: int = Field(ge=1, le=MAX_DEPTH)
    max_path_depth: int = Field(ge=1, le=MAX_DEPTH)


class InterpretRequest(Strict):
    question: Question
    ontology: ActiveOntology
    selected_entities: list[SelectedEntity] = Field(
        default_factory=list, max_length=MAX_SELECTED_ENTITIES
    )
    #: Earlier questions, oldest first (follow-ups only).
    conversation: list[ConversationTurn] = Field(
        default_factory=list, max_length=MAX_CONVERSATION_TURNS
    )
    earlier_entities: list[EarlierEntity] = Field(
        default_factory=list, max_length=MAX_EARLIER_ENTITIES
    )
    limits: InterpretLimits

    @model_validator(mode="after")
    def _known_turn_entities(self) -> InterpretRequest:
        known = {e.ref for e in self.selected_entities} | {e.ref for e in self.earlier_entities}
        for turn in self.conversation:
            if not set(turn.entities) <= known:
                raise ValueError("a conversation turn refers to an entity that is not listed")
        return self


class InterpretResponse(Strict):
    intent: RetrievalIntent
    metadata: SuggestionMetadata


class RetrievalCompleteness(Strict):
    complete: bool
    truncated: bool
    limitations: list[Annotated[str, StringConstraints(max_length=1000)]] = Field(
        default_factory=list, max_length=50
    )


class SynthesizeRequest(Strict):
    question: Question
    #: Earlier questions, oldest first (follow-ups only): context for what
    #: the question refers to, never evidence.
    earlier_questions: list[Question] = Field(
        default_factory=list, max_length=MAX_CONVERSATION_TURNS
    )
    #: The bounded evidence context built by MindGraph Core (references,
    #: types and properties; no MindGraph ids).
    context: dict[str, Any]
    retrieval: RetrievalCompleteness


class SynthesizedAnswer(Strict):
    answer: Annotated[str, StringConstraints(min_length=1, max_length=MAX_ANSWER_CHARS)]
    evidence_refs: list[Annotated[str, StringConstraints(max_length=32)]] = Field(
        default_factory=list, max_length=300
    )
    insufficient_evidence: bool = False
    warnings: list[ShortText] = Field(default_factory=list, max_length=MAX_WARNINGS)


class SynthesizeResponse(Strict):
    answer: SynthesizedAnswer
    metadata: SuggestionMetadata
