"""The retained outbox: what was sent, kept until the peer acknowledges it.

Every outgoing message is stored with the exact ciphertext and RPC identity it was
first sent with (§3.7 answers a resend with the ORIGINAL bytes, never a new
message). This mixin transmits those records, replays them for a resend, and
rewrites them into deletions when their content must go. Mixed into
``SecretChatManager``, which owns ``_client``, ``_storage`` and ``_inflight``.
"""

from __future__ import annotations

from telethon.extensions import BinaryReader
from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, sequence
from .chat import ChatState
from .errors import ResendUnsatisfiable
from .schema import secret_tl as tl

__all__ = ["RetainedOutbox"]


class RetainedOutbox:
    @staticmethod
    def _retained_random_id(item):
        if "random_id" in item:
            return item["random_id"]
        return sequence.unpack(item).message.random_id

    def _rewrite_retained_as_deletes(self, chat, random_ids):
        for item in self._storage.retained_out(chat.id):
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

    async def _transmit(self, chat, item):
        peer = types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash)
        if "frame" not in item:
            # Old stores never recorded the RPC identity, cipher, or file handle.
            # Guessing would turn an acknowledged ciphertext into a different send.
            failure = ResendUnsatisfiable(
                chat_id=chat.id,
                requested=(item["seq_no"], item["seq_no"]),
                retained_from=item["seq_no"],
            )
            failure.fatal = True
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
