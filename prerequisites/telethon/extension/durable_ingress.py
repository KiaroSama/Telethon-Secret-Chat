"""Versioned explicit durable-ingress contract supplied before connection.

This extension is opt-in and never opens a client, exports a session or changes
ordinary update processing. A contract owns protected durable custody and replay.
"""

from __future__ import annotations

import inspect

API_VERSION = 1


def configure(client, contract):
    if client.is_connected() or getattr(client, "_durable_ingress", None) is not None:
        raise ValueError("durable ingress must be configured exactly once before connection")
    if getattr(contract, "api_version", None) != API_VERSION:
        raise ValueError("unsupported durable ingress contract version")
    for name in (
        "bind",
        "restore",
        "capture_live",
        "capture_difference",
        "guard_requests",
        "guard_result",
        "session_state",
        "stop",
    ):
        if not callable(getattr(contract, name, None)):
            raise ValueError("durable ingress contract is incomplete")
    validator = getattr(contract, "validate_attachment", None)
    if not callable(validator):
        raise ValueError("durable ingress attachment cannot be validated")
    validator(client)
    client._durable_ingress = contract
    return client


async def invoke(client, name, *args):
    contract = getattr(client, "_durable_ingress", None)
    if contract is None:
        return None
    result = getattr(contract, name)(*args)
    return await result if inspect.isawaitable(result) else result
