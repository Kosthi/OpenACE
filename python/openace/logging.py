import logging
import os
import sys
from typing import Any, Dict, Optional

import structlog

def redact_sensitive_data(_, __, event_dict: Dict[str, Any]) -> Dict[str, Any]:
    """Redact sensitive keys from the event dict."""
    sensitive_keys = {"api_key", "token", "secret", "authorization", "password"}
    for key in list(event_dict.keys()):
        if any(sk in key.lower() for sk in sensitive_keys):
            event_dict[key] = "[REDACTED]"
    return event_dict


class _TracingRenderer:
    """Render structlog events in tracing-subscriber compatible format.

    Produces lines like::

        2026-02-20T18:56:04.566715Z  INFO openace.engine: search done count=10

    which visually matches Rust ``tracing_subscriber::fmt`` output.
    """

    _LEVEL_COLORS = {
        "TRACE": "\033[35m",   # magenta
        "DEBUG": "\033[34m",   # blue
        "INFO":  "\033[32m",   # green
        "WARN":  "\033[33m",   # yellow
        "ERROR": "\033[31m",   # red
    }
    _RESET = "\033[0m"
    _BOLD  = "\033[1m"
    _DIM   = "\033[2m"

    def __init__(self, colors: bool = True):
        self._colors = colors

    def __call__(self, _logger, _method_name, event_dict: Dict[str, Any]) -> str:
        ts = event_dict.pop("timestamp", "")
        level = event_dict.pop("level", "info").upper()
        if level == "WARNING":
            level = "WARN"
        event = event_dict.pop("event", "")
        # "_logger_name" is injected by _add_logger_name processor
        target = event_dict.pop("_logger_name", "")

        if self._colors:
            color = self._LEVEL_COLORS.get(level, "")
            level_str = f"{color}{level:>5}{self._RESET}"
            target_str = f" {self._DIM}{target}{self._RESET}:" if target else ""
            kv_parts = []
            for k, v in event_dict.items():
                kv_parts.append(f"{self._BOLD}{k}{self._RESET}={v}")
            kvs = " ".join(kv_parts)
        else:
            level_str = f"{level:>5}"
            target_str = f" {target}:" if target else ""
            kvs = " ".join(f"{k}={v}" for k, v in event_dict.items())

        line = f"{ts} {level_str}{target_str} {event}"
        if kvs:
            line += " " + kvs
        return line


def _add_logger_name(_, __, event_dict: Dict[str, Any]) -> Dict[str, Any]:
    """No-op placeholder — ``_logger_name`` is bound at logger creation time
    via :func:`get_logger`, so nothing needs to be extracted here.  Kept as a
    processor slot in case stdlib integration is added later.
    """
    return event_dict


def configure_logging(level: Optional[str] = None, log_format: Optional[str] = None):
    """Configure structlog for OpenACE.

    Can be called multiple times (e.g., from CLI after import-time defaults).
    Resets cached loggers so the new configuration takes effect.
    """
    # Reset any cached loggers from a previous configure() call so
    # reconfiguration (e.g., CLI --verbose after import-time default) works.
    structlog.reset_defaults()

    # Priority: argument > environment variable > default
    effective_level_str = (level or os.environ.get("OPENACE_LOG_LEVEL", "WARN")).upper()
    effective_format = (log_format or os.environ.get("OPENACE_LOG_FORMAT", "pretty")).lower()

    # Map string level to numeric level for filtering
    level_map = {
        "TRACE": logging.DEBUG,  # structlog has no TRACE; clamp to DEBUG
        "DEBUG": logging.DEBUG,
        "INFO": logging.INFO,
        "WARN": logging.WARNING,
        "WARNING": logging.WARNING,
        "ERROR": logging.ERROR,
        "CRITICAL": logging.CRITICAL,
    }
    numeric_level = level_map.get(effective_level_str, logging.WARNING)

    processors = [
        structlog.contextvars.merge_contextvars,
        structlog.processors.add_log_level,
        _add_logger_name,
        structlog.processors.StackInfoRenderer(),
        structlog.dev.set_exc_info,
        structlog.processors.TimeStamper(fmt="iso"),
        redact_sensitive_data,
    ]

    if effective_format == "json":
        processors.append(structlog.processors.JSONRenderer())
    else:
        colors = sys.stderr.isatty()
        processors.append(_TracingRenderer(colors=colors))

    structlog.configure(
        processors=processors,
        context_class=dict,
        logger_factory=structlog.PrintLoggerFactory(file=sys.stderr),
        wrapper_class=structlog.make_filtering_bound_logger(numeric_level),
        cache_logger_on_first_use=True,
    )

def get_logger(name: Optional[str] = None):
    """Get a structlog logger, binding the module name for the renderer."""
    logger = structlog.get_logger()
    if name:
        logger = logger.bind(_logger_name=name)
    return logger
