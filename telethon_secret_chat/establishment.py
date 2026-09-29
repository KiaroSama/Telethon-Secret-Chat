"""The initial exchange - protocol-reference.md §1.3-§1.6, the network half.

Requesting, accepting and completing a chat, the DH configuration and the
discard. Mixed into ``SecretChatManager``, which owns ``_client``, ``_chats``
and ``_storage``.
"""

from __future__ import annotations

import logging
import secrets
from typing import Optional

from telethon import errors as telethon_errors
from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, dh, framing, handshake
from .chat import ChatState, SecretChat
from .errors import ChatClosed, ChatNotReady, ParameterRejected, SecretChatError
from .events import ChatReady, ChatRequested
from .locking import serialized
from .host import ManagerHost

log = logging.getLogger("telethon_secret_chat")

__all__ = ["Establishment"]


class Establishment(ManagerHost):
    async def create(self, user):
        """Ask ``user`` for a secret chat.

        Returns: a ``ChatSnapshot`` in state ``requested``; ``ChatReady`` fires when
        the peer accepts.
        """
        g, p = await self._dh_config()
        secret = handshake.generate_secret()
        peer = await self._client.get_input_entity(user)
        self._creating += 1
        try:
            result = await self._client(
                functions.messages.RequestEncryptionRequest(
                    user_id=peer,
                    random_id=secrets.randbits(31),
                    g_a=handshake.public_value(g, secret, p).to_bytes(256, "big"),
                )
            )
        finally:
            self._creating -= 1
        chat = SecretChat(
            id=result.id,
            access_hash=result.access_hash,
            peer_user_id=getattr(result, "participant_id", None) or peer.user_id,
            is_outbound=True,
            admin_id=getattr(result, "admin_id", None),
            participant_id=getattr(result, "participant_id", None),
        )
        chat.dh_prime, chat.dh_g = p, g
        chat.handshake = {"secret": secret, "p": p, "g": g}
        try:
            self._save(chat)
        except BaseException:
            # Without its stored secret the chat can never be keyed; leaving it on
            # the server shows the peer a request that cannot complete.
            await self._discard_remote(chat.id)
            raise
        self._chats[chat.id] = chat
        early = self._early_encryption.pop(chat.id, None)
        if isinstance(result, types.EncryptedChat):
            await self._on_encryption(result)
        elif early is not None:
            await self._on_encryption(early)
        return chat.snapshot()

    @serialized
    async def accept(self, chat_id):
        """Accept a chat the peer requested (state ``pending``).

        Returns: a ``ChatSnapshot`` of the now ready chat; ``ChatReady`` fires too.
        Raises: UnknownChat; ChatNotReady if the chat is not awaiting acceptance;
        ChatClosed if the peer or another device already ended it.
        """
        chat = self._require(chat_id)
        if chat.state in (ChatState.READY, ChatState.REKEYING):
            return chat.snapshot()
        if chat.state is not ChatState.PENDING:
            chat.require_sendable()
            raise ChatNotReady(chat_id=chat_id, state=chat.state.value)
        pending = chat.handshake
        if "g_a" not in pending:
            raise ChatNotReady(chat_id=chat_id, state=chat.state.value)
        if "secret" not in pending:
            with self._atomic(chat):
                pending["secret"] = handshake.generate_secret()
        key = handshake.shared_key(
            peer_value=pending["g_a"], secret=pending["secret"], p=pending["p"], chat_id=chat_id
        )
        try:
            await self._client(
                functions.messages.AcceptEncryptionRequest(
                    peer=types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash),
                    g_b=handshake.public_value(
                        pending["g"], pending["secret"], pending["p"]
                    ).to_bytes(256, "big"),
                    key_fingerprint=crypto.key_fingerprint(key),
                )
            )
        except telethon_errors.EncryptionAlreadyAcceptedError:
            # Our g_b reached the server before a crash kept the commit below from
            # running; the persisted secret reproduces exactly the key the peer holds.
            pass
        except (
            telethon_errors.EncryptionDeclinedError,
            telethon_errors.EncryptionAlreadyDeclinedError,
            telethon_errors.EncryptionIdInvalidError,
        ):
            reason = "the request is no longer open on the server"
            self._close_local(chat, reason)
            raise ChatClosed(chat_id=chat_id, reason=reason) from None
        with self._atomic(chat):
            chat.adopt_key(key)
            chat.handshake = {}
        self._emit_ready(chat)
        await self._notify_layer(chat)
        return chat.snapshot()

    def _emit_ready(self, chat):
        # adopt_key has just run, so the fingerprint is set.
        assert chat.key_fingerprint is not None
        self._emit(
            ChatReady(chat.id, chat.peer_user_id, chat.key_fingerprint, chat.initial_key_hash)
        )

    @serialized
    async def _on_encryption(self, encrypted):
        chat: Optional[SecretChat]
        if isinstance(encrypted, types.EncryptedChatRequested):
            if encrypted.id in self._chats:
                return  # Duplicate requests cannot replace a keyed chat or a tombstone.
            g, p = await self._dh_config()
            g_a = dh.value_from_bytes(encrypted.g_a)
            try:
                dh.check_peer_value(g_a, p, chat_id=encrypted.id)
            except ParameterRejected:
                # Refused here, so tell the server too: otherwise the peer is left with
                # a request that can never complete.
                await self._discard_remote(encrypted.id)
                raise
            chat = SecretChat(
                id=encrypted.id,
                access_hash=encrypted.access_hash,
                peer_user_id=encrypted.admin_id,
                is_outbound=False,
                admin_id=encrypted.admin_id,
                participant_id=encrypted.participant_id,
            )
            chat.dh_prime, chat.dh_g = p, g
            chat.handshake = {"g_a": g_a, "p": p, "g": g}
            self._save(chat)
            self._chats[chat.id] = chat
            self._emit(ChatRequested(chat.id, encrypted.admin_id))
        elif isinstance(encrypted, types.EncryptedChat):
            chat = self._chats.get(encrypted.id)
            if chat is None:
                if self._creating and len(self._early_encryption) < 100:
                    self._early_encryption[encrypted.id] = encrypted
                return
            if chat.state is ChatState.CLOSED or chat.key is not None:
                return
            pending = chat.handshake
            if chat.state is ChatState.PENDING and "secret" not in pending:
                # Another device of this account accepted first; the chat is theirs, so
                # it ends here without discarding it on the server.
                self._close_local(chat, "the request was accepted by another device")
                return
            if "secret" not in pending:
                await self.close(chat.id, "the initial exchange cannot be recovered")
                return
            try:
                key = handshake.shared_key(
                    peer_value=dh.value_from_bytes(encrypted.g_a_or_b),
                    secret=pending["secret"],
                    p=pending["p"],
                    chat_id=chat.id,
                )
                handshake.verify_fingerprint(
                    chat_id=chat.id, key=key, claimed=encrypted.key_fingerprint
                )
            except SecretChatError as failure:
                await self.close(chat.id, failure.reason)
                raise
            with self._atomic(chat):
                chat.adopt_key(key)
                chat.handshake = {}
            self._emit_ready(chat)
            await self._notify_layer(chat)
        elif isinstance(encrypted, types.EncryptedChatDiscarded):
            chat = self._chats.get(encrypted.id)
            if chat is not None and chat.state is not ChatState.CLOSED:
                self._close_local(chat, "the peer discarded the chat")

    async def _notify_layer(self, chat):
        await self._send_action(chat, actions_module.notify_layer(framing.MAX_LAYER))

    async def _dh_config(self):
        config = await self._client(
            functions.messages.GetDhConfigRequest(version=0, random_length=0)
        )
        p = dh.value_from_bytes(config.p)
        dh.check_config(g=config.g, p=p)
        return config.g, p

    async def _discard_remote(self, chat_id):
        """Best effort: the local state is already what counts."""
        try:
            await self._client(
                functions.messages.DiscardEncryptionRequest(chat_id=chat_id, delete_history=False)
            )
        except Exception as failure:
            # The local close already happened; the peer only learns of it from this
            # call, so a failure is worth a warning. The type only, never the text.
            log.warning(
                "discardEncryption failed for chat %s: %s", chat_id, type(failure).__name__
            )
