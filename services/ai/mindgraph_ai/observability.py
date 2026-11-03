"""Structured logging with correlation ids.

Log records carry ids, sizes, durations and counts only: never prompts,
dataset samples, provider responses or API keys.
"""

from __future__ import annotations

import contextvars
import logging
import re
import uuid

request_id_var: contextvars.ContextVar[str] = contextvars.ContextVar("request_id", default="-")

_VALID_REQUEST_ID = re.compile(r"^[\x21-\x7e]{1,128}$")


def accept_request_id(value: str | None) -> str:
    """The caller's ``x-request-id`` if it is short visible ASCII, else a new id."""
    if value and _VALID_REQUEST_ID.match(value):
        return value
    return str(uuid.uuid4())


class _RequestIdFilter(logging.Filter):
    def filter(self, record: logging.LogRecord) -> bool:
        record.request_id = request_id_var.get()
        return True


def configure_logging(level: int = logging.INFO) -> None:
    """Configures ``mindgraph_ai`` loggers once (idempotent)."""
    logger = logging.getLogger("mindgraph_ai")
    if any(getattr(handler, "_mindgraph", False) for handler in logger.handlers):
        return
    handler = logging.StreamHandler()
    handler._mindgraph = True  # type: ignore[attr-defined]
    handler.addFilter(_RequestIdFilter())
    handler.setFormatter(
        logging.Formatter(
            "%(asctime)s %(levelname)s %(name)s request_id=%(request_id)s %(message)s"
        )
    )
    logger.addHandler(handler)
    logger.setLevel(level)
    logger.propagate = False
