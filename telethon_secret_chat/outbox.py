"""The retained outbox: what was sent, kept until the peer acknowledges it.

Every outgoing message is stored with the exact ciphertext and RPC identity it was
first sent with (§3.7 answers a resend with the ORIGINAL bytes, never a new
message). This mixin transmits those records, replays them for a resend, and
rewrites them into deletions when their content must go. Mixed into
``SecretChatManager``, which owns ``_client``, ``_storage`` and ``_inflight``.
"""

from __future__ import annotations

import logging
import secrets

from telethon.extensions import BinaryReader
from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, framing
from . import rekey as rekey_module
from . import sequence
from .chat import ChatState
from .errors import ManagerStopping, ResendUnsatisfiable, SecretChatError, SendPending
from .locking import serialized
from .schema import secret_tl as tl

log = logging.getLogger("telethon_secret_chat")

__all__ = ["RetainedOutbox"]


class RetainedOutbox:
    @staticmethod
    def _retained_random_id(item):
        if "random_id" in item:
            return item["random_id"]
        return sequence.unpack(item).message.random_id

    def _rewrite_retained_as_deletes(self, chat, random_ids):
        for item in self._storage.retained_out(chat.id):
            # §3.8 withdraws deleted MESSAGES. A service action has no content to
            # withdraw, and rewriting a pending rekey step or Resend request would
            # leave the exchange or the hole open for good.
            if item.get("method") == "service":
                continue
            if self._retained_random_id(item) not in random_ids:
                continue
            wrapper = sequence.unpack(item)
            wrapper.message = tl.DecryptedMessageService(
                random_id=wrapper.message.random_id,
                action=actions_module.delete_messages([wrapper.message.random_id]),
            )
            item.update(
                body=bytes(wrapper).hex(),
                frame=crypto.encrypt_frame(chat.key, bytes(wrapper), chat.out_x).hex(),
                method="service",
                file=None,
            )
            self._storage.queue_out(chat.id, item)

    def _forget_locally(self, chat, random_ids):
        """The one local deletion: retained content rewritten, then history."""
        with self._atomic(chat):
            self._rewrite_retained_as_deletes(chat, random_ids)
        self._remove_history(chat.id, random_ids)

    def _forget_everything_locally(self, chat):
        # plans/007: content records only; retained protocol actions keep their frames.
        with self._atomic(chat):
            self._rewrite_retained_as_deletes(chat, self._content_random_ids(chat))
        self._forget_history(chat.id)

    def _content_random_ids(self, chat):
        """Every retained message a history flush withdraws (never service actions)."""
        return {
            self._retained_random_id(item)
            for item in self._storage.retained_out(chat.id)
            if item.get("method") != "service"
        }

    async def _transmit(self, chat, item):
        peer = types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash)
        if "frame" not in item:
            # Old stores never recorded the RPC identity, cipher, or file handle.
            # Guessing would turn an acknowledged ciphertext into a different send.
            failure = ResendUnsatisfiable(
                chat_id=chat.id,
                requested=(item["seq_no"], item["seq_no"]),
                retained_from=item["seq_no"],
                fatal=True,
            )
            await self.close(chat.id, "legacy retained message has no original wire record")
            raise failure
        arguments = dict(peer=peer, random_id=item["random_id"], data=bytes.fromhex(item["frame"]))
        if item.get("method") == "file":
            with BinaryReader(bytes.fromhex(item["file"])) as reader:
                arguments["file"] = reader.tgread_object()
            request = functions.messages.SendEncryptedFileRequest(**arguments)
        elif item.get("method") == "service":
            request = functions.messages.SendEncryptedServiceRequest(**arguments)
        else:
            request = functions.messages.SendEncryptedRequest(**arguments)
        identity = (chat.id, item["seq_no"])
        self._inflight.add(identity)
        try:
            result = await self._client(request)
            with self._storage.transaction():
                # A nested receive may have acknowledged/deleted this record already.
                retained = self._storage.retained_out(chat.id)
                if any(
                    record["seq_no"] == item["seq_no"] and record.get("body") == item.get("body")
                    for record in retained
                ):
                    item = dict(item, pending=False)
                    attached = getattr(result, "file", None)
                    if isinstance(attached, types.EncryptedFile):
                        item["file"] = bytes(
                            types.InputEncryptedFile(
                                id=attached.id,
                                access_hash=attached.access_hash,
                            )
                        ).hex()
                    self._storage.queue_out(chat.id, item)
        finally:
            self._inflight.discard(identity)

    async def _request_due_resend(self, chat):
        """§3.7's one request per hole, sent until it is durably queued.

        ``sequence.accept`` marks the hole and records the span in the same commit;
        the mark is cleared only by the commit that queues the request. A failure in
        between leaves it due, so the next message or restart asks again rather than
        the hole waiting for ever behind a request that never left.
        """
        if chat.resend_due and chat.state is not ChatState.CLOSED:
            start, end = chat.resend_due
            # Part of the hole may have arrived since the span was decided, and this
            # side's echo may already have let the peer forget it; asking for it again
            # would be unsatisfiable and end the chat. Ask only for what is missing.
            start = max(start, 2 * chat.in_seq_no + start % 2)
            if start > end:
                chat.resend_due = None
                return
            await self._send_action(
                chat,
                actions_module.resend(start, end),
                after_prepare=lambda: setattr(chat, "resend_due", None),
            )

    async def _send_action(self, chat, action, *, after_prepare=None, encryption_key=None):
        # PFS counts every encrypted use, so service-only traffic reaches the
        # trigger too; the exchange's own actions never do (they would recurse).
        if not isinstance(action, rekey_module.PROTOCOL_ACTIONS):
            await self._rekey_if_due(chat)
        await self._send(
            chat,
            tl.DecryptedMessageService(random_id=secrets.randbits(63), action=action),
            after_prepare=after_prepare,
            encryption_key=encryption_key,
        )

    @serialized
    async def _send(self, chat, message, file=None, *, after_prepare=None, encryption_key=None):
        chat.require_sendable()
        if self._stopping:
            raise ManagerStopping()
        await self.retry_pending(chat.id)
        with self._atomic(chat):
            wrapper = framing.wrap(
                message,
                layer=framing.outgoing_layer(chat.layer),
                in_seq_no=framing.transform_in_seq_no(chat.in_seq_no, chat.is_outbound),
                out_seq_no=framing.transform_out_seq_no(chat.out_seq_no, chat.is_outbound),
            )
            body = bytes(wrapper)
            item = {
                "seq_no": wrapper.out_seq_no,
                "body": body.hex(),
                "frame": crypto.encrypt_frame(encryption_key or chat.key, body, chat.out_x).hex(),
                "random_id": message.random_id,
                "pending": True,
                "method": (
                    "file"
                    if file is not None
                    else (
                        "service"
                        if isinstance(
                            message, (tl.DecryptedMessageService, tl.DecryptedMessageService8)
                        )
                        else "message"
                    )
                ),
                "file": bytes(file).hex() if file is not None else None,
            }
            chat.out_seq_no += 1
            chat.messages_since_rekey += 1
            self._storage.queue_out(chat.id, item)
            if after_prepare is not None:
                after_prepare()
        await self._transmit_or_pending(chat, item)

    @serialized
    async def retry_pending(self, chat_id):
        chat = self._sendable(chat_id)
        for item in self._storage.retained_out(chat_id):
            if item.get("pending") and (chat_id, item["seq_no"]) not in self._inflight:
                await self._transmit_or_pending(chat, item)

    async def _transmit_or_pending(self, chat, item):
        """Transmit a committed record; a transport failure becomes ``SendPending``.

        The record is already durable and will be retried, so the caller must learn
        its id and must not send again. ``from None`` also keeps the frames holding the
        plaintext and key out of the traceback that reaches the application.
        """
        try:
            await self._transmit(chat, item)
        except SecretChatError:
            raise
        except Exception as failure:
            raise SendPending(
                chat_id=chat.id,
                random_id=self._retained_random_id(item),
                cause=type(failure).__name__,
            ) from None
