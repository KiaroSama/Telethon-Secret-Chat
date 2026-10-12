"""Finite authorization-wide ingress budgets and detached, content-hidden receipts."""

from dataclasses import dataclass, field
import math


@dataclass(frozen=True)
class IngressLimits:
    records: int
    bytes: int
    age_seconds: float
    batch_bytes: int = 1024 * 1024
    operation_seconds: float = 15

    def __post_init__(self):
        for name, ceiling in (
            ("records", 4096),
            ("bytes", 64 * 1024 * 1024),
            ("batch_bytes", 4 * 1024 * 1024),
        ):
            value = getattr(self, name)
            if type(value) is not int or not 0 < value <= ceiling:
                raise ValueError(f"{name} must be a positive integer no greater than {ceiling}")
        for name, ceiling in (("age_seconds", 30 * 86400), ("operation_seconds", 60)):
            value = getattr(self, name)
            if (
                type(value) not in (int, float)
                or not math.isfinite(value)
                or not 0 < value <= ceiling
            ):
                raise ValueError(f"{name} must be positive and finite, no greater than {ceiling}")
        if self.batch_bytes > self.bytes:
            raise ValueError("batch_bytes cannot exceed the aggregate byte limit")


@dataclass(frozen=True)
class IngressBatch:
    identity: str
    kind: str
    envelope: bytes = field(repr=False)
    before: dict = field(repr=False)
    after: dict = field(repr=False)
    cursor: int
    applied: tuple[int, ...] = ()
