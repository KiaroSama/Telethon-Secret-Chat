"""Auto-save: every message and file of every secret chat, kept on disk (spec 009).

Switched on for all chats with a folder the application gives, and remembered in the
manager's storage until switched off. Each chat gets ``<folder>/<chat_id>/`` holding
``messages.jsonl`` (one JSON record per line, appended and fsynced, never rewritten),
``files/`` (decrypted files) and, while a download is still owed, ``pending.json``.

Self-destructing messages are kept too: the owner's explicit decision (2026-09-30).
The folder therefore holds plaintext - protect it like the store. A saved record
never carries a file key; a download still owed keeps its reference (key included)
in ``pending.json`` only, removed once the file is on disk. A failure to save is
logged by its type and never breaks sending, receiving or delivery.
"""

from __future__ import annotations

import asyncio
import base64
import json
import logging
import mimetypes
import os
import re
import shutil
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

from . import files
from .media import MediaReference
from .schema import secret_tl as tl
from .schema.secret_tl import SecretTLObject

log = logging.getLogger("telethon_secret_chat")

SETTING = "auto_save_secret_chats"
FORMAT = 1
# Actions a person sees in a chat; the protocol's own (rekey, layer, resend, reads)
# are not part of the conversation.
KEPT_ACTIONS = frozenset(
    {
        "decryptedMessageActionSetMessageTTL",
        "decryptedMessageActionScreenshotMessages",
        "decryptedMessageActionDeleteMessages",
        "decryptedMessageActionFlushHistory",
    }
)
_UNSAFE = re.compile(r'[<>:"/\\|?*\x00-\x1f]')


def _plain(value):
    """A schema object as JSON-safe data; bytes (previews) as base64."""
    if isinstance(value, SecretTLObject):
        return {"_": value.TL_NAME, **{k: _plain(v) for k, v in vars(value).items()}}
    if isinstance(value, (list, tuple)):
        return [_plain(item) for item in value]
    if isinstance(value, bytes):
        return base64.b64encode(value).decode("ascii")
    return value


def describe(media) -> Optional[dict]:
    """The media without its one-time key: the saved file is already plaintext."""
    if media is None or isinstance(media, tl.DecryptedMessageMediaEmpty):
        return None
    plain = _plain(media)
    plain.pop("key", None)
    plain.pop("iv", None)
    return plain


def file_name(media, fallback: str = "") -> str:
    for attribute in getattr(media, "attributes", None) or ():
        if isinstance(attribute, tl.DocumentAttributeFilename) and attribute.file_name:
            fallback = attribute.file_name
    if not fallback:
        mime = getattr(media, "mime_type", "") or (
            "image/jpeg" if "Photo" in type(media).__name__ else ""
        )
        fallback = "file" + (mimetypes.guess_extension(mime) or ".bin")
    return _UNSAFE.sub("_", Path(fallback).name) or "file.bin"


def _restrict(path: Path) -> None:
    if os.name != "nt":
        os.chmod(path, 0o600)


class AutoSave:
    def __init__(self, manager):
        self._manager = manager
        self.folder: Optional[Path] = None  # set while auto-save is on
        self.saved_in: Optional[Path] = None  # kept after it is off, for reading and deleting
        self._tasks: set = set()

    # --- the switch ---------------------------------------------------------------

    def load(self) -> None:
        setting = self._manager._storage.load_setting(SETTING)
        if setting is None and self._manager._default_save_folder is not None:
            self.enable(self._manager._default_save_folder)
            return
        setting = setting or {}
        self.saved_in = Path(setting["folder"]) if setting.get("folder") else None
        self.folder = self.saved_in if setting.get("on") else None

    def enable(self, folder) -> None:
        self._manager._storage.save_setting(SETTING, {"folder": str(Path(folder)), "on": True})
        self.folder = self.saved_in = Path(folder)

    def disable(self) -> None:
        self._manager._storage.save_setting(
            SETTING,
            {"folder": str(self.saved_in) if self.saved_in is not None else None, "on": False},
        )
        self.folder = None

    # --- recording ----------------------------------------------------------------

    def received(self, event) -> None:
        if self.folder is None:
            return
        record = self._message(event.random_id, False, event.text, event.entities, event.ttl)
        record.update(reply_to=event.reply_to, media=describe(event.media))
        if self._guarded(event.chat_id, self._append, event.chat_id, record) and (
            event.media_reference is not None
        ):
            name = file_name(event.media)
            if self._guarded(
                event.chat_id,
                self._owe,
                event.chat_id,
                event.random_id,
                event.media_reference,
                name,
            ):
                self._download(event.chat_id, event.random_id, event.media_reference, name)

    def service_received(self, chat_id, action) -> None:
        self._service(chat_id, action, out=False)

    def sent(self, chat_id, message) -> None:
        """Every outgoing record, from the one place all sends pass (``outbox._send``)."""
        if self.folder is None:
            return
        action = getattr(message, "action", None)
        if action is not None:
            self._service(chat_id, action, out=True)
            return
        record = self._message(
            message.random_id,
            True,
            getattr(message, "message", "") or "",
            getattr(message, "entities", None),
            getattr(message, "ttl", 0) or 0,
        )
        record.update(
            reply_to=getattr(message, "reply_to_random_id", None),
            media=describe(getattr(message, "media", None)),
        )
        self._guarded(chat_id, self._append, chat_id, record)

    def sent_file(self, chat_id, random_id, source, name, start) -> None:
        """Keep the plaintext this side just sent; no download needed."""
        if self.folder is None:
            return
        self._guarded(chat_id, self._copy, chat_id, random_id, source, name, start)

    def forwarded(self, chat_id, random_id, source) -> None:
        """A forward reuses the server's file: fetch it once like a received one."""
        if self.folder is None:
            return
        reference = source if isinstance(source, MediaReference) else source.media_reference
        if reference is None:
            return
        name = file_name(reference.decoded())
        if self._guarded(chat_id, self._owe, chat_id, random_id, reference, name):
            self._download(chat_id, random_id, reference, name)

    # --- reading and deleting -----------------------------------------------------

    def read(self, chat_id) -> List[dict]:
        if self.saved_in is None:
            return []
        path = self.saved_in / str(chat_id) / "messages.jsonl"
        if not path.exists():
            return []
        records: Dict[int, dict] = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                record = json.loads(line)
            except ValueError:
                continue  # a line cut short by a crash; everything before it stands
            if record.get("type") == "file":
                if record.get("id") in records:
                    records[record["id"]]["file"] = record["path"]
            else:
                records[record["id"]] = record
        return list(records.values())

    def delete(self, chat_id) -> None:
        if self.saved_in is not None and (self.saved_in / str(chat_id)).exists():
            shutil.rmtree(self.saved_in / str(chat_id))

    # --- downloads ----------------------------------------------------------------

    def resume(self) -> None:
        """Downloads a stop interrupted: their references wait in ``pending.json``."""
        if self.folder is None or not self.folder.is_dir():
            return
        for directory in self.folder.iterdir():
            owed = directory / "pending.json"
            if not owed.is_file():
                continue
            try:
                for random_id, entry in json.loads(owed.read_text(encoding="utf-8")).items():
                    reference = MediaReference.from_dict(entry["reference"])
                    self._download(reference.chat_id, int(random_id), reference, entry["name"])
            except Exception as failure:
                log.warning("auto-save could not resume downloads: %s", type(failure).__name__)

    async def settle(self) -> None:
        while self._tasks:
            await asyncio.gather(*list(self._tasks), return_exceptions=True)

    async def cancel(self) -> None:
        tasks = list(self._tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)

    def _download(self, chat_id, random_id, reference, name) -> None:
        task = asyncio.ensure_future(self._fetch(chat_id, random_id, reference, name))
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)

    async def _fetch(self, chat_id, random_id, reference, name) -> None:
        target = self._files(chat_id) / f"{random_id}-{name}"
        try:
            await files.receive(self._manager, reference, target)
            _restrict(target)
            self._append(
                chat_id, {"v": FORMAT, "type": "file", "id": random_id, "path": str(target)}
            )
            self._settle_owed(chat_id, random_id)
        except asyncio.CancelledError:
            raise  # stop(): the reference stays owed and start() retries it
        except Exception as failure:
            log.warning("auto-save failed for chat %s: %s", chat_id, type(failure).__name__)

    # --- disk ---------------------------------------------------------------------

    def _message(self, random_id, out, text, entities, ttl) -> dict:
        return {
            "v": FORMAT,
            "type": "message",
            "id": random_id,
            "date": int(time.time()),
            "out": out,
            "text": text,
            "entities": _plain(list(entities or ())),
            "ttl": ttl,
        }

    def _service(self, chat_id, action, out) -> None:
        if self.folder is None or getattr(action, "TL_NAME", "") not in KEPT_ACTIONS:
            return
        record = {
            "v": FORMAT,
            "type": "service",
            "id": f"service-{time.time_ns()}",
            "date": int(time.time()),
            "out": out,
            "action": _plain(action),
        }
        self._guarded(chat_id, self._append, chat_id, record)

    def _guarded(self, chat_id, work, *args) -> bool:
        try:
            work(*args)
            return True
        except Exception as failure:
            log.warning("auto-save failed for chat %s: %s", chat_id, type(failure).__name__)
            return False

    def _chat_dir(self, chat_id) -> Path:
        assert self.folder is not None
        directory = self.folder / str(chat_id)
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        return directory

    def _files(self, chat_id) -> Path:
        directory = self._chat_dir(chat_id) / "files"
        directory.mkdir(exist_ok=True, mode=0o700)
        return directory

    def _append(self, chat_id, record) -> None:
        path = self._chat_dir(chat_id) / "messages.jsonl"
        new = not path.exists()
        with open(path, "a", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        if new:
            _restrict(path)

    def _copy(self, chat_id, random_id, source, name, start) -> None:
        target = self._files(chat_id) / f"{random_id}-{_UNSAFE.sub('_', Path(name).name)}"
        if isinstance(source, (bytes, bytearray, memoryview)):
            target.write_bytes(bytes(source))
        elif hasattr(source, "read"):
            source.seek(start)
            with open(target, "wb") as out:
                shutil.copyfileobj(source, out)
        else:
            shutil.copyfile(source, target)
        _restrict(target)
        self._append(chat_id, {"v": FORMAT, "type": "file", "id": random_id, "path": str(target)})

    def _owe(self, chat_id, random_id, reference, name) -> None:
        owed = self._read_owed(chat_id)
        owed[str(random_id)] = {"reference": reference.to_dict(), "name": name}
        self._write_owed(chat_id, owed)

    def _settle_owed(self, chat_id, random_id) -> None:
        owed = self._read_owed(chat_id)
        if owed.pop(str(random_id), None) is not None:
            self._write_owed(chat_id, owed)

    def _read_owed(self, chat_id) -> Dict[str, Any]:
        path = self._chat_dir(chat_id) / "pending.json"
        return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}

    def _write_owed(self, chat_id, owed) -> None:
        path = self._chat_dir(chat_id) / "pending.json"
        if not owed:
            path.unlink(missing_ok=True)
            return
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(owed), encoding="utf-8")
        _restrict(temporary)
        os.replace(temporary, path)
