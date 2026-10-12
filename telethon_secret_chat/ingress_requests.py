"""Inspect acknowledgment requests without allowing wrapper bypasses."""

from telethon.tl import functions

from .ingress_store import IngressRefused

WRAPPERS = (
    "InvokeWithoutUpdatesRequest",
    "InvokeWithLayerRequest",
    "InvokeAfterMsgRequest",
    "InvokeAfterMsgsRequest",
    "InvokeWithTakeoutRequest",
    "InitConnectionRequest",
)


def unwrap(request):
    for _ in range(8):
        if type(request).__name__ not in WRAPPERS:
            if hasattr(request, "query"):
                raise IngressRefused()
            return request
        request = request.query
    raise IngressRefused()


def frontier_of(request):
    request = unwrap(request)
    if isinstance(request, functions.updates.GetDifferenceRequest):
        return request.qts
    if isinstance(request, functions.messages.ReceivedQueueRequest):
        return request.max_qts
    return None
