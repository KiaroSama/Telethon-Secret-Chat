"""Telegram MTProto 2.0 end-to-end encryption (secret chats) for Telethon.

The public surface is exactly ``contracts/public-api.md`` and nothing else. The
protocol internals - ``crypto``, ``dh``, ``framing``, ``sequence``, ``rekey``, and
the generated schema - are importable by their module path for tests, and are not a
supported surface: they may move without notice.

There is deliberately no way here to supply a key, skip a check, or lower the layer.
The one thing this package sells is that the checks ran.
"""

from .chat import ChatState, SecretChat
from .errors import (
    ChatClosed,
    ChatNotReady,
    LayerUnsupported,
    ParameterRejected,
    ResendUnsatisfiable,
    SecretChatError,
    StorageRequired,
)
from .events import (
    ChatClosedEvent,
    ChatReady,
    ChatRequested,
    DecryptFailed,
    MessageAcknowledged,
    MessageReceived,
    ServiceActionReceived,
)
from .files import CAPTIONLESS_KINDS, MEDIA_KINDS
from .manager import SecretChatManager
from .storage import FileStorage, MemoryStorage, StorageBackend

__version__ = "0.0.1"

__all__ = [
    # what an application constructs and holds
    "SecretChatManager",
    "SecretChat",
    "ChatState",
    # storage - required, never defaulted (FR-014)
    "StorageBackend",
    "MemoryStorage",
    "FileStorage",
    # the media kinds `send_file` accepts
    "MEDIA_KINDS",
    "CAPTIONLESS_KINDS",
    # errors (contracts §4)
    "SecretChatError",
    "ParameterRejected",
    "ChatNotReady",
    "ChatClosed",
    "StorageRequired",
    "LayerUnsupported",
    "ResendUnsatisfiable",
    # events (contracts §3)
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
]
