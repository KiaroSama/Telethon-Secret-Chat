# Stable key visualization

The stored first-key hash is SHA-1(key)[0..16] concatenated with SHA-256(key)[0..20].
It remains stable across rekey and is distinct from the signed 64-bit fingerprint
used to verify DH agreement. The visualization layer maps this 36-byte value to
the Telegram-style color pattern and provides the original image representation.
Typed width checks reject invalid inputs before rendering.

`ChatSnapshot::key_hash` exposes a detached visualization hash, not shared-key
bytes. Rekey must never replace it with the newest key's hash. Exact native tests
cover known pixels, representation, shape and rekey stability; the burned official
capture remains an independent protocol fixture. See [primitives](../rust-primitives.md).
