# Typed media metadata

The Rust media layer supports photo, video, animation, video note, audio, voice,
sticker and document kinds. FileSource owns bytes or a bounded stream declaration;
path inputs are inspected before sequence reservation. The caller supplies codec
metadata such as dimensions, duration, title/performer/waveform and thumbnails.
The library does not run a media-transcoding process.

Schema source is `schema/end-to-end.tl`. Document constructors carry thumbnail,
MIME, size, key/IV, attributes and caption. Attribute families and optional fields
are validated before effects. Oversized/truncated streams, inconsistent reference
fingerprints and invalid caption policies fail before upload/reservation where
possible. Known-size streamed encryption verifies that the actual input matches
its declared size. Files above the old 2000 MB ceiling require peer layer 143.

`SendFileOptions` and `media::Metadata` are documented in [managed API](../rust-manager.md).
HTML export has separate Rust ICC/scaling/thumbnail compatibility rules described
in [exporter](../rust-export.md); its exact fixtures do not claim that every
platform's external decoder will produce identical pixels for arbitrary inputs.
