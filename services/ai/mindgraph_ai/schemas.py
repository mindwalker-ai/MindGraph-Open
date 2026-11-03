"""Explicit request and response schemas (Pydantic v2).

The suggestion schema mirrors MindGraph Core's contract exactly: the
ontology has the shape of ``POST /api/v1/graphs/{id}/ontology`` and the
mapping is the M4 ``IngestionMapping``. Unknown fields are rejected, every
list is bounded and every confidence lies in ``0.0..=1.0``.

Rust validation in MindGraph Core remains authoritative; this layer stops
malformed model output before it leaves the AI service.
"""

from __future__ import annotations

from typing import Annotated, Any, Literal

from pydantic import BaseModel, ConfigDict, Field, StringConstraints

#: Machine names of types and properties: the same rule as mindgraph-domain.
MachineName = Annotated[str, StringConstraints(pattern=r"^[A-Za-z][A-Za-z0-9_]{0,63}$")]
PropertyType = Literal["string", "integer", "float", "boolean", "datetime", "string_array"]
Confidence = Annotated[float, Field(ge=0.0, le=1.0, allow_inf_nan=False)]
ShortText = Annotated[str, StringConstraints(max_length=300)]

MAX_ENTITY_TYPES = 25
MAX_RELATIONSHIP_TYPES = 50
MAX_PROPERTIES = 100
MAX_WARNINGS = 20
#: Datasets modelled together in one request (MindGraph Core's bound).
MAX_DATASETS = 10

DatasetName = Annotated[str, StringConstraints(min_length=1, max_length=300)]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


# --- Ontology and mapping drafts ------------------------------------------------


class PropertyDraft(Strict):
    name: MachineName
    type: PropertyType
    required: bool = False


class EntityTypeDraft(Strict):
    name: MachineName
    display_name: Annotated[str, StringConstraints(max_length=120)] | None = None
    description: Annotated[str, StringConstraints(max_length=2000)] | None = None
    properties: list[PropertyDraft] = Field(default_factory=list, max_length=MAX_PROPERTIES)


class RelationshipTypeDraft(Strict):
    name: MachineName
    source_entity_type: MachineName
    target_entity_type: MachineName
    properties: list[PropertyDraft] = Field(default_factory=list, max_length=MAX_PROPERTIES)


class OntologyDraft(Strict):
    """A proposed ontology version: existing types kept, new types added."""

    entity_types: list[EntityTypeDraft] = Field(max_length=MAX_ENTITY_TYPES)
    relationship_types: list[RelationshipTypeDraft] = Field(
        default_factory=list, max_length=MAX_RELATIONSHIP_TYPES
    )


class FieldMapping(Strict):
    source_field: Annotated[str, StringConstraints(min_length=1, max_length=255)]
    property: MachineName


class EntityMapping(Strict):
    alias: Annotated[str, StringConstraints(min_length=1, max_length=64)]
    entity_type: MachineName
    mode: Literal["create_or_match", "match_only"] = "create_or_match"
    identity: list[FieldMapping] = Field(min_length=1, max_length=10)
    properties: list[FieldMapping] = Field(default_factory=list, max_length=MAX_PROPERTIES)


class RelationshipMapping(Strict):
    relationship_type: MachineName
    source: Annotated[str, StringConstraints(min_length=1, max_length=64)]
    target: Annotated[str, StringConstraints(min_length=1, max_length=64)]
    properties: list[FieldMapping] = Field(default_factory=list, max_length=MAX_PROPERTIES)


class MappingDraft(Strict):
    """The M4 ``IngestionMapping`` contract, unchanged."""

    entities: list[EntityMapping] = Field(min_length=1, max_length=MAX_ENTITY_TYPES)
    relationships: list[RelationshipMapping] = Field(
        default_factory=list, max_length=MAX_RELATIONSHIP_TYPES
    )


class TypeAnnotation(Strict):
    name: Annotated[str, StringConstraints(min_length=1, max_length=64)]
    confidence: Confidence
    reason: ShortText = ""
    #: Set by MindGraph Core from the real ontology; model claims are ignored.
    existing: bool = False


class Annotations(Strict):
    entities: list[TypeAnnotation] = Field(default_factory=list, max_length=MAX_ENTITY_TYPES)
    relationships: list[TypeAnnotation] = Field(
        default_factory=list, max_length=MAX_RELATIONSHIP_TYPES
    )


class DatasetMappingDraft(Strict):
    """The M4 mapping of one dataset, named as in the request."""

    dataset: DatasetName
    mapping: MappingDraft


class GraphBuilderSuggestion(Strict):
    ontology: OntologyDraft
    mappings: list[DatasetMappingDraft] = Field(min_length=1, max_length=MAX_DATASETS)
    confidence: Confidence
    annotations: Annotations = Field(default_factory=Annotations)
    warnings: list[ShortText] = Field(default_factory=list, max_length=MAX_WARNINGS)


# --- Internal request / response -------------------------------------------------


class FieldProfile(Strict):
    name: Annotated[str, StringConstraints(min_length=1, max_length=255)]
    inferred_type: PropertyType
    nullable: bool
    non_null_count: int = Field(ge=0)
    sample_values: list[Annotated[str, StringConstraints(max_length=200)]] = Field(max_length=5)


class DatasetProfile(Strict):
    """The bounded M4 profile. Never the source file."""

    record_count: int = Field(ge=0)
    fields: list[FieldProfile] = Field(max_length=500)
    sample_rows: list[dict[str, Any]] = Field(max_length=20)


# The active ontology is whatever the graph already has: MindGraph Core does
# not bound properties per type, so these models do not reuse the suggestion
# limits. The 512 KiB request cap bounds the whole payload.
MAX_ACTIVE_TYPES = 1000
MAX_ACTIVE_PROPERTIES = 10_000


class ActiveEntityType(Strict):
    name: MachineName
    display_name: Annotated[str, StringConstraints(max_length=120)] | None = None
    description: Annotated[str, StringConstraints(max_length=2000)] | None = None
    properties: list[PropertyDraft] = Field(default_factory=list, max_length=MAX_ACTIVE_PROPERTIES)


class ActiveRelationshipType(Strict):
    name: MachineName
    source_entity_type: MachineName
    target_entity_type: MachineName
    properties: list[PropertyDraft] = Field(default_factory=list, max_length=MAX_ACTIVE_PROPERTIES)


class ActiveOntology(Strict):
    """The pinned active ontology (existing graphs may exceed suggestion bounds)."""

    version: int = Field(ge=1)
    entity_types: list[ActiveEntityType] = Field(default_factory=list, max_length=MAX_ACTIVE_TYPES)
    relationship_types: list[ActiveRelationshipType] = Field(
        default_factory=list, max_length=MAX_ACTIVE_TYPES
    )


class RepairContext(Strict):
    previous: GraphBuilderSuggestion
    errors: list[Annotated[str, StringConstraints(min_length=1, max_length=500)]] = Field(
        min_length=1, max_length=50
    )


class DatasetInput(Strict):
    """One analysed dataset: a label unique in the request and its profile."""

    name: DatasetName
    profile: DatasetProfile


class SuggestRequest(Strict):
    datasets: list[DatasetInput] = Field(min_length=1, max_length=MAX_DATASETS)
    active_ontology: ActiveOntology
    repair: RepairContext | None = None


class SuggestionMetadata(Strict):
    prompt_version: str
    provider: str
    model: str


class SuggestResponse(Strict):
    suggestion: GraphBuilderSuggestion
    metadata: SuggestionMetadata


class Health(Strict):
    status: str
    service: str
    version: str


class Readiness(Strict):
    status: Literal["ready", "not_configured"]
    configured: bool
    provider: str | None = None
    model: str | None = None
    reason: str | None = None
