"""decryptedMessageLayer and what wraps a message - protocol-reference.md §3.1-§3.3,
plus the layer arithmetic of §7 and the ``seq_no`` transform of §3.4.

The wrapper and the layer belong together: the layer a message announces is the one
§7.4 computes from what the peer has claimed, and getting it wrong makes a message
unparseable at the far end rather than merely wrong.

Three numbers here are worth stating because §8.7 measured the archived package
getting each one wrong: the remote layer starts at **46** and not 101, it only ever
**rises**, and the layer we send at is **clamped to [73, 144]** rather than copied
from the peer.
"""

from __future__ import annotations

import os

from telethon.extensions import BinaryReader

from .errors import LayerUnsupported, MessageRejected
from .schema import secret_tl as tl

__all__ = [
    "INITIAL_REMOTE_LAYER",
    "MIN_LAYER",
    "MAX_LAYER",
    "RANDOM_BYTES_LENGTH",
    "wrap",
    "unwrap",
    "outgoing_layer",
    "raise_remote_layer",
    "require_supported_layer",
    "transform_in_seq_no",
    "transform_out_seq_no",
    "raw_seq_no",
]

# §7.2: "When the secret chat is first created, this value should be initialized to
# 46." It is an assumption about a peer that has not spoken yet, not a claim it made.
INITIAL_REMOTE_LAYER = 46

# §3.2: the floor for any layer appearing in a decryptedMessageLayer.
MIN_WRAPPER_LAYER = 46

# §7.1 and §7.5: 73 is where MTProto 2.0 becomes mandatory in both directions, and
# this package implements nothing below it (FR-017).
MIN_LAYER = 73

# §7.1: TDLib's `Current`. Announcing higher would promise constructors we cannot
# parse - §7.3: announcing a layer implies being able to parse everything it
# introduced.
MAX_LAYER = 144

# §3.3: the schema states no minimum; TDLib always sends exactly 31 and the
# reference marks anything shorter UNVERIFIED. §8.3 measured the archived package
# sending 15, 19 or 23 bytes and choosing between them with the `random` module.
RANDOM_BYTES_LENGTH = 31


def wrap(message, *, layer: int, in_seq_no: int, out_seq_no: int) -> tl.DecryptedMessageLayer:
    """§3.1: put a message inside ``decryptedMessageLayer``.

    Service messages go through here too - §3.1: "Note that any service messages in
    secret chats must also increment the seq_no", which means they carry the same
    wrapper and the same counters as anything else.
    """
    return tl.DecryptedMessageLayer(
        random_bytes=os.urandom(RANDOM_BYTES_LENGTH),
        layer=layer,
        in_seq_no=in_seq_no,
        out_seq_no=out_seq_no,
        message=message,
    )


def unwrap(body: bytes, *, chat_id: int | None = None) -> tl.DecryptedMessageLayer:
    """Parse a decrypted body into its wrapper. Raises ``MessageRejected``.

    Anything that is not a well-formed wrapper at layer 46 or above is refused
    rather than interpreted: a bare ``DecryptedMessage`` with no wrapper carries no
    ``seq_no`` at all, so accepting one would silently skip every §3.5 and §3.6
    check the rest of this package exists to perform.
    """
    try:
        with BinaryReader(body) as reader:
            decoded = tl.read_object(reader)
            if reader.tell_position() != len(body):
                raise ValueError("trailing bytes after the wrapper")
    except tl.UnknownConstructor:
        raise MessageRejected(
            chat_id=chat_id,
            reason="the message names a constructor this schema does not define",
        ) from None
    except Exception:
        # Deliberately broad, and deliberately silent about the cause: a parser
        # error names an offset, and an offset is the length oracle §2.7 warns
        # about. `from None` keeps the underlying traceback out of the chain.
        raise MessageRejected(chat_id=chat_id, reason="the message could not be parsed") from None

    if not isinstance(decoded, tl.DecryptedMessageLayer):
        raise MessageRejected(
            chat_id=chat_id, reason="the message is not wrapped in decryptedMessageLayer"
        )
    if decoded.layer < MIN_WRAPPER_LAYER:
        raise MessageRejected(
            chat_id=chat_id,
            reason="the message announces a layer below the documented minimum",
        )
    allowed = (
        tl.DecryptedMessage,
        tl.DecryptedMessage_1f814f1f,
        tl.DecryptedMessage_204d3878,
        tl.DecryptedMessage_36b091de,
        tl.DecryptedMessageService,
        tl.DecryptedMessageService8,
    )
    if not isinstance(decoded.message, allowed):
        raise MessageRejected(chat_id=chat_id, reason="the wrapper does not contain a message")
    return decoded


def raise_remote_layer(stored: int, announced: int) -> int:
    """§7.2: "This remote layer value must always be updated immediately after
    receiving any packet containing information of an upper layer."

    Upper only. TDLib never lowers it, and §3.6 rejects a message claiming less than
    the stored value - a peer that could walk its layer down could walk it below 73
    and out of MTProto 2.0.
    """
    return max(stored, announced)


def outgoing_layer(remote_layer: int) -> int:
    """§7.4: ``clamp(his_layer, 73, 144)``.

    TDLib's ``current_layer()`` starts at Current (144), lowers it to the peer's if
    the peer is lower, then raises it back to Default (73) if that dropped below.
    The lower clamp is not politeness: below 73 the protocol is MTProto 1.0, which
    this package does not implement.
    """
    return max(MIN_LAYER, min(remote_layer, MAX_LAYER))


def require_supported_layer(*, chat_id: int, peer_layer: int) -> None:
    """FR-017: a peer that has announced below 73 is refused, with the reason stated.

    Only for a layer the peer actually CLAIMED. The initial 46 of §7.2 is this
    side's assumption about a peer that has not spoken yet, and refusing on it would
    refuse every chat before its first NotifyLayer arrived.
    """
    if peer_layer < MIN_LAYER:
        raise LayerUnsupported(chat_id=chat_id, peer_layer=peer_layer)


# --- §3.4, the mirroring guard ------------------------------------------------
#
# "They must be protected from mirroring before being sent to the remote client by
# transformation according to formula 2*raw_seq_no+x, where x is 0 or 1". The table:
#
#   chat initiated by | in_seq_no x | out_seq_no x
#   sender            | 0           | 1
#   recipient         | 1           | 0
#
# "chat initiated by" is about the SENDER of the message being written, so for our
# own outgoing messages it is `is_outbound` - whether this side called
# requestEncryption. The peer's messages carry the mirror image, which is what makes
# the parity check of §3.4 able to detect a message reflected back at us.


def transform_in_seq_no(raw: int, is_outbound: bool) -> int:
    return 2 * raw + (0 if is_outbound else 1)


def transform_out_seq_no(raw: int, is_outbound: bool) -> int:
    return 2 * raw + (1 if is_outbound else 0)


def raw_seq_no(wire: int) -> int:
    """Back to the counter. §3.7 does the same to a resend span - TDLib "divides by
    2 on receipt"."""
    return wire // 2
