"""AI Graph Builder: ontology and mapping suggestions for one dataset (M5)."""

from mindgraph_ai.graph_builder.prompts import PROMPT_VERSION, SYSTEM_PROMPT, build_user_prompt
from mindgraph_ai.graph_builder.service import GraphBuilderService

__all__ = ["PROMPT_VERSION", "SYSTEM_PROMPT", "GraphBuilderService", "build_user_prompt"]
