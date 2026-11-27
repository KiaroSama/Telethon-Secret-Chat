# Design note: caller-supplied media metadata, thumbnails, and bytes/stream input

Design spike, 2026-09-29. The schema facts and the proposed signature are settled here;
the live check (what an official client shows) was run on 2026-09-29 and passed (below).

## Today

`files.attributes_for(kind, file_name)` sends `DocumentAttributeImageSize(w=0, h=0)`,
`DocumentAttributeVideo(round_message, duration=0, w=0, h=0)`,
`DocumentAttributeAudio(voice, duration=0)` and `DocumentAttributeSticker(alt="",
stickerset=InputStickerSetEmpty())`; `files.send` builds the media with `thumb=b""`,
`thumb_w=0`, `thumb_h=0`. The package holds no codec by design, and the caller has no way to
pass the values it may know (the consumer runs ffmpeg). Input is a path only.

## Schema facts (`telethon_secret_chat/schema/end-to-end.tl`)

The newest form of each constructor, which is the one the package sends:

| Constructor | Fields a caller could supply |
|---|---|
| `decryptedMessageMediaDocument#6abd9782` | `thumb:bytes thumb_w:int thumb_h:int` (plus `mime_type`, `size:long`, `attributes`, `caption`) |
| `documentAttributeImageSize#6c37c15c` | `w:int h:int` |
| `documentAttributeVideo#ef02ce6` | `flags.0 round_message`, `duration:int w:int h:int` |
| `documentAttributeAudio#9852f9c6` | `flags.10 voice`, `duration:int`, `flags.0 title`, `flags.1 performer`, `flags.2 waveform:bytes` |
| `documentAttributeSticker#3a556302` | `alt:string stickerset:InputStickerSet` (`inputStickerSetShortName` or `inputStickerSetEmpty`) |

**Layer gating.** The schema file carries no per-constructor layer, so which layer introduced
`waveform`, `title` and `performer` is not recorded here (UNVERIFIED). It does not change the
design: these are fields of the constructors the package already sends at every outgoing
layer (73-144), so supplying values changes no constructor and needs no new gating. The only
layer rule on this path stays the existing one: files over 2000 MB need the peer at 143
(§7.1).

**Thumbnail size.** TDLib's `inputThumbnail` documentation (`td_api.tl`, tdlib/td commit
`42e6a5259551178d1dab54a22ad96d14bd906e20`, line 5801) asks for JPEG (or WEBP for stickers),
"less than 200 KB in size", width and height that "usually shouldn't exceed 320". Whether an
official client refuses a larger thumbnail in a secret chat is UNVERIFIED; the design adopts
TDLib's numbers as validation limits. The waveform is the "5-bit format" TDLib documents for
voice notes (line 620).

## Proposed signature

```python
await send_file(
    chat_id, source, *,
    file_name=None,            # required when source is bytes or a stream
    caption="", mime_type=None, kind=None, reply_to=None,
    duration=None,             # int seconds >= 0: video, video_note, audio, voice_note
    width=None, height=None,   # int >= 0: photo, video, video_note, animation, sticker
    thumbnail=None,            # bytes, JPEG (WEBP for sticker), < 200 KB
    thumbnail_size=None,       # (w, h), each 1..320; required with thumbnail
    waveform=None,             # bytes: voice_note only
    title=None, performer=None,  # str: audio only
    sticker_alt=None,          # str: sticker only
)
```

- A value given for a kind it does not apply to is refused (`ValueError`), never dropped
  silently; negative numbers and oversize thumbnails are refused before anything is uploaded.
- Omitted values keep today's zeros and empty thumbnail.
- **Source rule.** `source` is a path (today's behaviour), `bytes`, or a binary stream with a
  known size. For bytes and streams `file_name` is required: it is the mime guess input
  (`files.guess_mime`) and the `DocumentAttributeFilename`. The kind check needs the first
  `ogg_tags.HEADER_BYTES` (64 KiB): taken from the bytes, or read from the stream and then
  chained back in front of it for the upload (`files.EncryptingReader` encrypts from any
  readable source, so no temp file is written).

## Live check (passed 2026-09-29)

For the operator, once, with a temporary local change reverted afterwards (never committed):
send one video with real `duration`, `width`, `height` and a small JPEG thumbnail, one voice
note with `duration` and a waveform, and one sticker with `InputStickerSetEmpty`; record on
the official client: the duration shown, whether the video is sized before download, whether
a preview appears before download, whether the voice note shows its waveform, and whether the
sticker renders as a sticker or as an image.

Result, official Android client, layer 144 chat: the video (`duration=3, w=320, h=240`, a
90x68 JPEG thumb) showed its preview and `0:03` before download and was sized as a landscape
rectangle; the voice note (`voice=True, duration=3`, a 100-sample 5-bit waveform) showed its
waveform and duration; the webp with `DocumentAttributeSticker` + `InputStickerSetEmpty` +
`ImageSize(512, 512)` rendered as a sticker, not an image.

## Acceptance criteria for the feature

- Unit: each argument lands in the right attribute or media field; a value for the wrong kind,
  a negative number, a thumbnail over 200 KB or over 320 px, and a thumbnail without its size
  are refused before upload.
- Unit: bytes and stream sources produce the same ciphertext and attributes as the same
  content from a path.
- Live: `tests/interop/test_live_media.py` asks the operator the questions of the live check
  above and records the answers, not only "opened".
