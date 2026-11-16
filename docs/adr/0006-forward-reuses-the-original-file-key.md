# 0006. Forward original encrypted file references

Status: accepted; carried into the Rust rewrite.

A MediaReference binds the encrypted file, complete validated media TL, DC, size and file-key fingerprint. Forwarding verifies it before reserving a message, sends an InputEncryptedFile reference and preserves its file key/IV; it does not re-upload ciphertext or invent a replacement key. Captionless sticker/video-note contracts remain enforced. Reusing a file key shares access to the same encrypted file; that key does not authenticate file contents.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
