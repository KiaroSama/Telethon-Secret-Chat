"""Telegram MTProto 2.0 end-to-end encryption (secret chats) for Telethon.

The public surface is exactly ``__all__`` below and nothing else. The
protocol internals - ``crypto``, ``dh``, ``framing``, ``sequence``, ``rekey``, and
the generated schema - are importable by their module path for tests, and are not a
supported surface: they may move without notice.

There is deliberately no way here to supply a key, skip a check, or lower the layer.
The one thing this package sells is that the checks ran.

API reference: ``help(SecretChatManager)``; design: ``docs/architecture.md``.
"""

from importlib import metadata as _metadata

from .chat import ChatSnapshot, ChatState, SecretChat
from .errors import (
    ChatClosed,
    ChatNotReady,
    LayerUnsupported,
    ManagerStopping,
    ParameterRejected,
    ResendUnsatisfiable,
    SecretChatError,
    SendPending,
    StorageRequired,
    StoreCorrupt,
    UnknownChat,
)
from .events import (
    ChatClosedEvent,
    ChatReady,
    ChatRequested,
    DecryptFailed,
    MessageAcknowledged,
    MessageReceived,
    SendFailed,
    ServiceActionReceived,
)

# Imported for its side effect of binding `telethon_secret_chat.ogg_tags`,
# deliberately NOT in __all__: telling a voice note from a track is how
# `send_file` picks a default, not a promise this package makes to a
# caller. `test_the_package_exports_exactly_the_contract` is what keeps
# that line honest, and it caught this being widened by accident.
from . import ogg_tags  # noqa: F401
from .files import CAPTIONLESS_KINDS, MEDIA_KINDS
from .manager import SecretChatManager
from .media import MediaReference
from .visualization import PALETTE, KeyVisualization, key_visualization
from .storage import FileStorage, MemoryStorage, StorageBackend
from .protected import ProtectedFileStorage
from .transient_work import TransientLimits, TransientRefused, TransientCleanupIncomplete
from .transient import TransientSecretChatManager

try:
    # One source of truth: the installed distribution's metadata (pyproject.toml).
    __version__ = _metadata.version("kiaro-telethon-secret-chat")
except _metadata.PackageNotFoundError:  # a source tree that was never installed
    __version__ = "0+unknown"

__all__ = [
    # what an application constructs and holds
    "SecretChatManager",
    "TransientSecretChatManager",
    "TransientLimits",
    "TransientRefused",
    "TransientCleanupIncomplete",
    "ProtectedFileStorage",
    "SecretChat",
    "ChatSnapshot",
    "ChatState",
    # storage - required, never defaulted
    "StorageBackend",
    "MemoryStorage",
    "FileStorage",
    # the media kinds `send_file` accepts
    "MEDIA_KINDS",
    "CAPTIONLESS_KINDS",
    # errors
    "SecretChatError",
    "ParameterRejected",
    "ChatNotReady",
    "ChatClosed",
    "StorageRequired",
    "LayerUnsupported",
    "ResendUnsatisfiable",
    "StoreCorrupt",
    "UnknownChat",
    "ManagerStopping",
    "SendPending",
    # events (docs/architecture.md §4)
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
    "SendFailed",
    "MediaReference",
    "KeyVisualization",
    "key_visualization",
    "PALETTE",
]
