"""Multi-provider LLM client with retry and rate-limit handling."""

from __future__ import annotations

import logging
import os
import time
from dataclasses import dataclass, field
from typing import Protocol

from eval.swebench.config import LLMConfig

logger = logging.getLogger(__name__)

_MAX_RETRIES = 5
_RETRY_BASE_DELAY = 2.0  # seconds


@dataclass
class TokenUsage:
    """Accumulated token usage statistics."""

    prompt_tokens: int = 0
    completion_tokens: int = 0

    @property
    def total_tokens(self) -> int:
        return self.prompt_tokens + self.completion_tokens


class LLMClient(Protocol):
    """Unified LLM client interface."""

    def generate(self, prompt: str, *, system: str = "") -> str: ...


class OpenAIClient:
    """LLM client backed by the OpenAI API."""

    def __init__(self, config: LLMConfig) -> None:
        import openai

        api_key = os.environ.get(config.api_key_env)
        if not api_key:
            raise RuntimeError(
                f"Environment variable {config.api_key_env} is not set"
            )
        client_kwargs: dict = {"api_key": api_key}
        if config.base_url:
            client_kwargs["base_url"] = config.base_url
        self._client = openai.OpenAI(**client_kwargs)
        self._model = config.model
        self._temperature = config.temperature
        self._max_tokens = config.max_tokens
        self.usage = TokenUsage()

    def generate(self, prompt: str, *, system: str = "") -> str:
        messages = []
        if system:
            messages.append({"role": "system", "content": system})
        messages.append({"role": "user", "content": prompt})

        for attempt in range(_MAX_RETRIES):
            try:
                resp = self._client.chat.completions.create(
                    model=self._model,
                    messages=messages,
                    temperature=self._temperature,
                    max_tokens=self._max_tokens,
                )
                if resp.usage:
                    self.usage.prompt_tokens += resp.usage.prompt_tokens
                    self.usage.completion_tokens += resp.usage.completion_tokens
                return resp.choices[0].message.content or ""
            except Exception as e:
                if attempt == _MAX_RETRIES - 1:
                    raise
                delay = _RETRY_BASE_DELAY * (2 ** attempt)
                logger.warning(
                    "OpenAI request failed (attempt %d/%d): %s — retrying in %.1fs",
                    attempt + 1, _MAX_RETRIES, e, delay,
                )
                time.sleep(delay)
        return ""  # unreachable


class AnthropicClient:
    """LLM client backed by the Anthropic API."""

    def __init__(self, config: LLMConfig) -> None:
        import anthropic

        api_key = os.environ.get(config.api_key_env)
        if not api_key:
            raise RuntimeError(
                f"Environment variable {config.api_key_env} is not set"
            )
        client_kwargs: dict = {"api_key": api_key}
        if config.base_url:
            client_kwargs["base_url"] = config.base_url
        self._client = anthropic.Anthropic(**client_kwargs)
        self._model = config.model
        self._temperature = config.temperature
        self._max_tokens = config.max_tokens
        self.usage = TokenUsage()

    def generate(self, prompt: str, *, system: str = "") -> str:
        for attempt in range(_MAX_RETRIES):
            try:
                kwargs: dict = {
                    "model": self._model,
                    "max_tokens": self._max_tokens,
                    "messages": [{"role": "user", "content": prompt}],
                }
                if self._temperature > 0:
                    kwargs["temperature"] = self._temperature
                if system:
                    kwargs["system"] = system

                resp = self._client.messages.create(**kwargs)
                if resp.usage:
                    self.usage.prompt_tokens += resp.usage.input_tokens
                    self.usage.completion_tokens += resp.usage.output_tokens
                return resp.content[0].text if resp.content else ""
            except Exception as e:
                if attempt == _MAX_RETRIES - 1:
                    raise
                delay = _RETRY_BASE_DELAY * (2 ** attempt)
                logger.warning(
                    "Anthropic request failed (attempt %d/%d): %s — retrying in %.1fs",
                    attempt + 1, _MAX_RETRIES, e, delay,
                )
                time.sleep(delay)
        return ""  # unreachable


def create_llm_client(config: LLMConfig) -> OpenAIClient | AnthropicClient:
    """Factory to create the right LLM client from config."""
    if config.provider == "openai":
        return OpenAIClient(config)
    elif config.provider == "anthropic":
        return AnthropicClient(config)
    else:
        raise ValueError(f"Unknown LLM provider: {config.provider!r}")
