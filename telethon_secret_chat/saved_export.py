"""Validated offline boundary around the unchanged shared Desktop exporter."""

from __future__ import annotations

import logging
from pathlib import Path

from telethon.tl import types

from .tdexport.fetch import ExportWriter
from .tdexport.html_and_json import HtmlAndJsonWriter
from .tdexport.html_writer import HtmlWriter
from .tdexport.json_writer import JsonWriter
from .tdexport.secret_saved import export_saved_chat
from .tdexport.settings import MAX_FILE_SIZE, Format, MediaSettings, Settings

log = logging.getLogger("telethon_secret_chat")
_FORMATS = {"html": Format.Html, "json": Format.Json, "both": Format.HtmlAndJson}
_MEDIA = {
    "photo": MediaSettings.Type.Photo,
    "video": MediaSettings.Type.Video,
    "voice_message": MediaSettings.Type.VoiceMessage,
    "video_message": MediaSettings.Type.VideoMessage,
    "sticker": MediaSettings.Type.Sticker,
    "gif": MediaSettings.Type.GIF,
    "file": MediaSettings.Type.File,
}


def export(
    records,
    path,
    *,
    self_user,
    peer_user,
    source_folder=None,
    format="html",
    media_types=None,
    size_limit=8 * 1024 * 1024,
    date_from=0,
    date_till=0,
    force_sub_path=False,
):
    if not isinstance(format, str) or format not in _FORMATS:
        raise ValueError("format must be html, json or both")
    for user in (self_user, peer_user):
        if not isinstance(user, types.User) or type(user.id) is not int or user.id <= 0:
            raise ValueError("participants must be Telethon Users with positive ids")
    if type(size_limit) is not int or not 0 <= size_limit <= MAX_FILE_SIZE:
        raise ValueError("size_limit must be an integer between 0 and 4000 MiB")
    if any(type(value) is not int or value < 0 for value in (date_from, date_till)):
        raise ValueError("date bounds must be nonnegative Unix timestamps")
    if date_till and date_till <= date_from:
        raise ValueError("date_till must be greater than date_from")
    if type(force_sub_path) is not bool:
        raise ValueError("force_sub_path must be a bool")
    selected = MediaSettings.default_types()
    if media_types is not None:
        if isinstance(media_types, (str, bytes)):
            raise ValueError("media_types must be an iterable of media group names")
        selected = MediaSettings.Type(0)
        try:
            for name in media_types:
                selected |= _MEDIA[name]
        except (KeyError, TypeError):
            raise ValueError("media_types contains an invalid media group") from None
    if not isinstance(path, (str, Path)) or not str(path).strip():
        raise ValueError("path must be a nonempty filesystem path")
    destination = Path(path).resolve()
    if source_folder is not None:
        source = Path(source_folder).resolve()
        if destination == source or destination in source.parents or source in destination.parents:
            raise ValueError("export destination must not overlap the saved source")
    settings = Settings(
        path=str(destination),
        format=_FORMATS[format],
        media=MediaSettings(types=selected, size_limit=size_limit),
        single_peer_from=date_from,
        single_peer_till=date_till,
        force_sub_path=force_sub_path,
    )
    writer: ExportWriter
    if format == "json":
        writer = JsonWriter()
    elif format == "html":
        writer = HtmlWriter()
    else:
        writer = HtmlAndJsonWriter(HtmlWriter(), JsonWriter())
    try:
        result = export_saved_chat(records, settings, writer, self_user, peer_user)
    except Exception as failure:
        log.warning("saved export failed: %s", type(failure).__name__)
        raise OSError("Saved export failed; saved messages were not changed") from None
    finally:
        _close_writer(writer)
    log.info("Saved export finished: messages=%d files=%d", result.messages, result.files)
    return result


def _close_writer(writer):
    # The unchanged port has no failure-close protocol; retain ownership here.
    for child in getattr(writer, "_writers", [writer]):
        handle = getattr(child, "_chat", None) or getattr(child, "_output", None)
        if handle is not None:
            try:
                handle.close()
            except OSError as failure:
                log.warning("export handle cleanup failed: %s", type(failure).__name__)
