from __future__ import annotations

from typing import Any, Optional


class OpenBrainError(Exception):
    def __init__(self, code: str, message: str, *, status: Optional[int] = None, details: Any = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = status
        self.details = details

