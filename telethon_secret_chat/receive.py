"""The receive path: update dispatch, decryption, sequence checks and delivery.

A received message is committed with its sequence effects and placed in the
durable mailbox in one transaction; delivery to the application drains that
mailbox. Mixed into ``SecretChatManager``, which owns ``_chats``, ``_storage``,
``_history`` and ``_delivering``.
"""

from __future__ import annotations

import collections
import logging

from telethon.extensions import BinaryReader
from telethon.tl import types

from . import actions as actions_module
from . import crypto, framing
from . import rekey as rekey_module
from . import sequence
from .chat import ChatState
from .errors import SecretChatError
from .events import DecryptFailed, MessageAcknowledged, MessageReceived, ServiceActionReceived
from .locking import serialized
from .schema import secret_tl as tl

log = logging.getLogger("telethon_secret_chat")

__all__ = ["Receiving"]


class Receiving:
    async def _on_update(self, update):
        # No await between here and the per-chat lock. Telethon dispatches updates as
        # concurrent tasks; reaching the lock synchronously is what keeps them FIFO per
        # chat (tests/unit/test_replay_and_gap.py, concurrent-dispatch test).
        try:
            if isinstance(update, types.UpdateEncryption):
                await self._on_encryption(update.chat)
            elif isinstance(update, types.UpdateNewEncryptedMessage):
                await self._on_encrypted_message(update.message)
        except SecretChatError as failure:
            self._emit(DecryptFailed(failure.chat_id or 0, getattr(failure, "reason", "refused")))
        except Exception:
            log.error("secret-chat update failed; no received data or exception text is logged")
            item = getattr(update, "message", getattr(update, "chat", None))
            chat_id = getattr(item, "chat_id", getattr(item, "id", 0))
            self._emit(
                DecryptFailed(chat_id, "update processing failed; durable work may need retry")
            )

    @serialized
    async def _on_encrypted_message(self, message):
        chat = self._chats.get(message.chat_id)
        if chat is None or chat.key is None or chat.state is ChatState.CLOSED:
            self._emit(DecryptFailed(message.chat_id, "no usable key is held for this chat"))
            return
        answer, acknowledgements, switched = [], [], False
        try:
            named = rekey_module.select_key(
                chat, int.from_bytes(message.bytes[:8], "little", signed=True)
            )
            if named is None:
                self._emit(DecryptFailed(chat.id, "no key this chat holds matches the frame"))
                return
            # A fingerprint is routing metadata, never proof that the sender knows a key.
            body = crypto.decrypt_frame(named, message.bytes, chat.in_x, chat_id=chat.id)
            wrapper = framing.unwrap(body, chat_id=chat.id)
            with self._atomic(chat):
                if not sequence.preflight(chat, wrapper, self._storage):
                    return
                inner = wrapper.message
                action = getattr(inner, "action", None)
                if isinstance(action, tl.DecryptedMessageActionResend):
                    answer = sequence.answer_resend(
                        chat, self._storage, action.start_seq_no, action.end_seq_no
                    )
                    inner.action = tl.DecryptedMessageActionNoop()
                previous_ack = chat.peer_in_seq_no
                accepted = sequence.accept(chat, wrapper, self._storage, envelope=message)
                if named is chat.pending_key:
                    rekey_module.adopt_new_key(chat, named)
                    chat.transition_to(ChatState.READY)
                    chat.new_key_confirmed = True
                    switched = True
                elif named is chat.key and chat.previous_key is not None:
                    chat.new_key_confirmed = True
                chat.messages_since_rekey += 1
                if chat.peer_in_seq_no > previous_ack:
                    ceiling = framing.transform_out_seq_no(
                        chat.peer_in_seq_no - 1, chat.is_outbound
                    )
                    for item in self._storage.retained_out(chat.id):
                        if item["seq_no"] <= ceiling:
                            acknowledgements.append(
                                MessageAcknowledged(
                                    chat.id,
                                    item["seq_no"],
                                    [self._retained_random_id(item)],
                                )
                            )
                sequence.forget_acknowledged(chat, self._storage, chat.peer_in_seq_no)
                # In the commit, where the gap state is already final: retiring after the
                # drain left a RequestKey delivered from the mailbox facing a held key.
                rekey_module.retire_previous_key_if_settled(chat)
                # The request is consumed by this commit, so its answer has to be due
                # in it too - after the acknowledgement drop, which may have covered the
                # very span asked for. Pending, `retry_pending` owns delivery and a
                # network failure below cannot lose it (§3.7, one request per hole).
                for record in answer:
                    self._storage.queue_out(chat.id, dict(record, pending=True))
                chat.pending_deliveries.extend(sequence.pack(item) for item in accepted.ready)
        except SecretChatError as failure:
            if failure.fatal:
                await self.close(chat.id, failure.reason)
            self._emit(DecryptFailed(chat.id, getattr(failure, "reason", "refused")))
            return
        for event in acknowledgements:
            self._emit(event)
        # Local delivery first: what this commit accepted reaches the application even
        # when the network steps below fail.
        await self._drain_deliveries(chat)
        if chat.state is ChatState.CLOSED:
            return
        if answer:
            await self.retry_pending(chat.id)
        await self._request_due_resend(chat)
        if switched:
            await self._send_action(chat, tl.DecryptedMessageActionNoop())

    @serialized
    async def _drain_deliveries(self, chat):
        if chat.id in self._delivering:
            return
        self._delivering.add(chat.id)
        try:
            while chat.pending_deliveries and chat.state is not ChatState.CLOSED:
                item = chat.pending_deliveries[0]
                try:
                    wrapper = sequence.unpack(item)
                except Exception:
                    # It parsed once, on receipt; a record that no longer does must
                    # not hold every later message behind it.
                    wrapper = None
                    self._emit(DecryptFailed(chat.id, "a stored message could not be read"))
                if wrapper is not None:
                    await self._deliver(chat, wrapper, None)
                if chat.state is ChatState.CLOSED:
                    break  # Closing scrubbed the record and the mailbox; do not write it back.
                with self._atomic(chat):
                    if chat.pending_deliveries and chat.pending_deliveries[0] == item:
                        chat.pending_deliveries.pop(0)
        finally:
            self._delivering.discard(chat.id)

    async def _deliver(self, chat, wrapper, envelope):
        inner = wrapper.message
        if isinstance(inner, (tl.DecryptedMessageService, tl.DecryptedMessageService8)):
            outcome = await actions_module.handle(self, chat, inner.action)
            self._emit(
                ServiceActionReceived(chat.id, type(inner.action).__name__, inner.action, outcome)
            )
            return
        attached = getattr(wrapper, "_tsc_file", None)
        if attached:
            with BinaryReader(bytes.fromhex(attached)) as reader:
                attached = reader.tgread_object()
        event = MessageReceived(
            chat_id=chat.id,
            random_id=getattr(inner, "random_id", 0),
            seq_no=wrapper.out_seq_no,
            text=getattr(inner, "message", "") or "",
            entities=getattr(inner, "entities", None),
            ttl=getattr(inner, "ttl", 0) or 0,
            media=getattr(inner, "media", None),
            file=attached,
            reply_to=getattr(inner, "reply_to_random_id", None),
        )
        history = self._history.setdefault(chat.id, collections.deque(maxlen=self._history_limit))
        seen = self._history_ids.setdefault(chat.id, set())
        if event.random_id not in seen and self._history_limit:
            if len(history) == history.maxlen:
                seen.discard(history[0].random_id)
            history.append(event)
            seen.add(event.random_id)
        self._emit(event)

    def _remove_history(self, chat_id, random_ids):
        kept = [
            item for item in self._history.get(chat_id, ()) if item.random_id not in random_ids
        ]
        self._history[chat_id] = collections.deque(kept, maxlen=self._history_limit)
        self._history_ids[chat_id] = {item.random_id for item in kept}

    def _forget_history(self, chat_id):
        self._history.pop(chat_id, None)
        self._history_ids.pop(chat_id, None)
