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
            event_dict[key] = "********"
    return event_dict

def configure_logging(level: Optional[str] = None, log_format: Optional[str] = None):
    """Configure structlog for OpenACE."""
    
    # Priority: argument > environment variable > default
    effective_level_str = (level or os.environ.get("OPENACE_LOG_LEVEL", "WARN")).upper()
    effective_format = (log_format or os.environ.get("OPENACE_LOG_FORMAT", "pretty")).lower()

    # Map string level to numeric level for filtering
    level_map = {
        "TRACE": 5, # Custom
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
        structlog.processors.StackInfoRenderer(),
        structlog.dev.set_exc_info,
        structlog.processors.TimeStamper(fmt="iso"),
        redact_sensitive_data,
    ]

    if effective_format == "json":
        processors.append(structlog.processors.JSONRenderer())
    else:
        processors.append(structlog.dev.ConsoleRenderer(colors=True))

    structlog.configure(
        processors=processors,
        context_class=dict,
        logger_factory=structlog.PrintLoggerFactory(file=sys.stderr),
        wrapper_class=structlog.make_filtering_bound_logger(numeric_level),
        cache_logger_on_first_use=True,
    )

def get_logger(name: Optional[str] = None):
    """Get a structlog logger."""
    return structlog.get_logger(name)
