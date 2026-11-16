# Deterministic thumbnail inputs

These synthetic JPEG inputs preserve the `_jpeg` patterns and Pillow 13 source
encoding used by the original `test_tdexport_thumbs.py` suite. They are decoder
inputs, not cached thumbnail outputs. `cmyk.source.jpg` is a solid CMYK image;
`density.source.jpg` embeds a fixed sRGB profile and 72 dpi JFIF density.

The Rust tests independently exercise the complete decoder, color transform,
scaler, encoder, metadata and filesystem pipeline. The published Desktop SHA-256
and the existing Qt Adobe RGB fixture are checked without input-specific paths
in the implementation.
