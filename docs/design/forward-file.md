# Forwarding encrypted files

`media::MediaReference` persists complete validated file-media TL alongside file
ID/access hash/DC/fingerprint. It rejects wrong versions, negative sizes, trailing
TL and mismatched keys. Forwarding validates before sequence reservation and uses
the original encrypted file/key/IV without uploading again. Downloading obeys the
reference DC and verifies metadata before exposing output. See [managed media API](../rust-manager.md).

Caption overrides respect media kind: stickers and video notes have no caption.
Copies do not change an already encrypted file's identity. Anyone given the file
key can decrypt the same file; forwarding is not re-encryption. The fingerprint
binds the key and does not authenticate file contents. The old official-client
experiment is historical; native live verification is [explicitly opt-in](../rust-live-tests.md).
