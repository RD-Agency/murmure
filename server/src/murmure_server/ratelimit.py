# SPDX-License-Identifier: AGPL-3.0-or-later
"""Limitation de débit simple, en mémoire, par clé (seau à jetons).

Suffisant pour une instance unique ; pour plusieurs réplicas, limiter au niveau du reverse proxy.
"""

from __future__ import annotations

import math
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass


@dataclass(slots=True)
class _Bucket:
    tokens: float
    updated: float


class RateLimiter:
    def __init__(self, per_minute: int, clock: Callable[[], float] = time.monotonic) -> None:
        self.capacity = float(per_minute)
        self.rate = per_minute / 60.0  # jetons par seconde
        self._clock = clock
        self._buckets: dict[str, _Bucket] = {}
        self._lock = threading.Lock()

    @property
    def enabled(self) -> bool:
        return self.capacity > 0

    def check(self, key: str) -> float | None:
        """Consomme un jeton. Renvoie None si autorisé, sinon le délai d'attente en secondes."""
        if not self.enabled:
            return None
        now = self._clock()
        with self._lock:
            bucket = self._buckets.get(key)
            if bucket is None:
                bucket = self._buckets[key] = _Bucket(tokens=self.capacity, updated=now)
            bucket.tokens = min(self.capacity, bucket.tokens + (now - bucket.updated) * self.rate)
            bucket.updated = now
            if bucket.tokens >= 1:
                bucket.tokens -= 1
                return None
            return (1 - bucket.tokens) / self.rate

    @staticmethod
    def retry_after_header(delay: float) -> str:
        return str(max(1, math.ceil(delay)))
