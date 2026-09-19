"""MTProto 2.0 key derivation and the message frame - protocol-reference.md §2.

Serialization and length prefix (§2.1), padding (§2.2), `msg_key` (§2.3), the
`aes_key`/`aes_iv` KDF (§2.4), the `x = 0 / x = 8` split (§2.5), the outer frame
(§2.6) and the receive-side checks (§2.7).
"""
