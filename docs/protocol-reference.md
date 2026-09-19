# Secret Chat Protocol Reference

Engineering reference for MTProto 2.0 end-to-end encryption, written so that a spec and an
implementation can cite it. Constitution Principle II makes the published protocol the
specification; this file is the working index into that specification.

**How to read a claim here.** Every normative statement carries its source inline:

- `[DOC:<page>]` — `https://core.telegram.org/<page>`, followed by the sentence actually read.
- `[TD:<path>:<line>]` — TDLib `master` at `https://github.com/tdlib/td`, file path and line as of
  retrieval 2026-09-19, followed by what the code does there.
- **UNVERIFIED** — not established from either source. Never treat as settled; a spec decision or
  an oracle test must close it.

Section numbers are stable. Cite as `protocol-reference.md §2.3`.

---

## §0. Sources

| Tag | Source | Retrieved |
|---|---|---|
| `DOC:api/end-to-end` | End-to-End Encryption, Secret Chats | 2026-09-19 |
| `DOC:api/end-to-end/seq_no` | Sequence numbers in Secret Chats | 2026-09-19 |
| `DOC:api/end-to-end/pfs` | Perfect Forward Secrecy in Secret Chats | 2026-09-19 |
| `DOC:mtproto/security_guidelines` | Security guidelines for developers | 2026-09-19 |
| `DOC:schema/end-to-end` | End-to-End TL schema, **Layer 223** | 2026-09-19 |
| `TD:*` | TDLib `master`, Boost Software License 1.0 | 2026-09-19 |

`https://core.telegram.org/api/end-to-end/files` **does not exist** (the server answers "The page
has not been saved"). All file rules live in the parent page; see §6.

TDLib is the reference oracle (Constitution Principle I). Logic ported from it must be marked at
the site with the file it came from (Principle V).

---

## §1. Chat establishment

### §1.1 `messages.getDhConfig`

```
messages.getDhConfig#26cf8950 version:int random_length:int = messages.DhConfig;
messages.dhConfig#2c221edd g:int p:bytes version:int random:bytes = messages.DhConfig;
messages.dhConfigNotModified#c0e24635 random:bytes = messages.DhConfig;
```
`[DOC:method/messages.getDhConfig]`

`[DOC:api/end-to-end]`: "Executing this method before each new key generation procedure is of vital
importance." Caching is allowed by version: if the client's stored version is current the server
answers `messages.dhConfigNotModified` and the cached `(g, p)` stay in force.

`random_length > 0` asks the server for extra entropy. `[DOC:api/end-to-end]`: "using the server's
random sequence in its raw form may be unsafe, it must be combined with a client sequence." Passing
`random_length = 0` and using local CSPRNG entropy avoids the hazard entirely.

### §1.2 DH parameter checks a client MUST perform

These are the documented conditions. Nothing here is folklore; each is quoted.

**On `p`** — `[DOC:api/end-to-end]` and `[DOC:mtproto/security_guidelines]`: "The client is expected
to check whether p is a safe 2048-bit prime (meaning that both p and (p-1)/2 are prime, and that
2^2047 < p < 2^2048)".

1. `2^2047 < p < 2^2048` (i.e. exactly 2048 bits).
2. `p` is prime.
3. `(p-1)/2` is prime.

**On `g`** — same source: "g generates a cyclic subgroup of prime order (p-1)/2, i.e. is a quadratic
residue mod p. Since g is always equal to 2, 3, 4, 5, 6 or 7, this is easily done using quadratic
reciprocity law, yielding a simple condition on p mod 4g".

| `g` | required condition |
|---|---|
| 2 | `p mod 8 == 7` |
| 3 | `p mod 3 == 2` |
| 4 | none |
| 5 | `p mod 5 == 1` or `4` |
| 6 | `p mod 24 == 19` or `23` |
| 7 | `p mod 7 == 3`, `5` or `6` |

`[TD:td/mtproto/DhHandshake.cpp:21-93]` `DhHandshake::check_config` implements exactly this: the
2048-bit range check (line 23 comment "check that 2^2047 <= p < 2^2048"), the `switch (g_int)`
modulus table (lines 39-58, same six cases and same residues), then `prime.is_prime(ctx)` and
`half_prime.is_prime(ctx)` (lines 73, 83) under the comment "check whether p is a safe prime". A
`DhCallback` caches the verdict per prime so the primality test runs once (`callback->add_good_prime`,
line 90).

`[DOC:mtproto/security_guidelines]` permits a shortcut: "one might initially run only 15
Miller--Rabin iterations ... with error probability not exceeding one billionth, and do more
iterations in the background later", and permits a compiled-in table of known-good `(g, p)` pairs.

**On `g`, `g_a`, `g_b`** — `[DOC:api/end-to-end]`: "Both clients are to check that g, g_a and g_b are
greater than one and smaller than p-1. We recommend checking that g_a and g_b are between
2^{2048-64} and p - 2^{2048-64} as well."

4. `1 < g < p-1`, `1 < g_a < p-1`, `1 < g_b < p-1` — required.
5. `2^1984 <= g_a <= p - 2^1984`, same for `g_b` — recommended by the docs, **enforced** by TDLib:
   `[TD:td/mtproto/DhHandshake.cpp:95-125]` `dh_check` returns
   `Status::Error("g^a or g^b is not between 2^{2048-64} and dh_prime - 2^{2048-64}")`.

The checks apply to every DH value that ever arrives, including the rekey values of §4.

### §1.3 `messages.requestEncryption` — side A

```
messages.requestEncryption#f64daf43 user_id:InputUser random_id:int g_a:bytes = EncryptedChat;
```
`[DOC:method/messages.requestEncryption]`

`[DOC:api/end-to-end]`: "Client A computes a 2048-bit number a (using sufficient entropy or the
server's random; see above) and executes messages.requestEncryption after passing in
g_a := pow(g, a) mod dh_prime."

A must retain `a` until the chat completes; it is the only way to finish the exchange. B receives
`updateEncryption` with `encryptedChatRequested` **on every authorized device**, and
`[DOC:api/end-to-end]`: "The user must be shown basic information about User A and must be prompted
to accept or reject the request."

### §1.4 `messages.acceptEncryption` — side B

```
messages.acceptEncryption#3dbc0415 peer:InputEncryptedChat g_b:bytes key_fingerprint:long = EncryptedChat;
```
`[DOC:method/messages.acceptEncryption]`

B generates `b` under the same rules as `a`, then `[DOC:api/end-to-end]`: "it can immediately
generate the final shared key: key = (pow(g_a, b) mod dh_prime). If key length < 256 bytes, add
several leading zero bytes as padding so that the key is exactly 256 bytes long."

`[TD:td/mtproto/DhHandshake.cpp:218-222]` `gen_key()` does the padding with
`get_g_ab().to_binary(2048 / 8)` — fixed-width 256-byte big-endian, left-padded with zeros.

After B calls `acceptEncryption`, `[DOC:api/end-to-end]`: "For all of Client B's authorized devices,
except the current one, updateEncryption updates are sent with the constructor
encryptedChatDiscarded." This is the mechanism behind Constitution Principle II's "a secret chat is
bound to the authorization that performed the DH exchange".

A then receives `updateEncryption` with `encryptedChat` carrying `g_a_or_b` and `key_fingerprint`,
computes `key = pow(g_b, a) mod dh_prime` with the same 256-byte padding, and
`[DOC:api/end-to-end]`: "If the fingerprint for the received key is identical to the one that was
passed to encryptedChat, incoming messages can be sent and processed. Otherwise,
messages.discardEncryption must be executed and the user notified."

### §1.5 `key_fingerprint`

`[DOC:api/end-to-end]`: "Its fingerprint, key_fingerprint, is equal to the 64 last bits of
SHA1 (key)."

Two notes are attached to that sentence in the same source and both matter:

- "Note 1: in this particular case SHA1 is used here even for MTProto 2.0 secret chats."
- "Note 2: this fingerprint is used as a sanity check for the key exchange procedure to detect bugs
  when developing client software — it is not connected to the key visualization".

Integer encoding: `[TD:td/mtproto/DhHandshake.cpp:225-229]` `calc_key_id` computes `sha1(auth_key)`
into a 20-byte buffer and returns `as<int64>(auth_key_sha1.raw + 12)` — the **last 8 bytes, read as
a little-endian signed 64-bit integer**. That same value is the `auth_key_id` prefixed to every
encrypted message (§2.6), so one routine serves both uses.

### §1.6 Who computes what

| Step | A (initiator; TDLib "creator") | B (acceptor) |
|---|---|---|
| DH config | fetch, validate `p`/`g` (§1.2) | fetch, validate `p`/`g` |
| secret | generate `a`, 2048 bits | generate `b`, 2048 bits |
| public | `g_a = g^a mod p`, sent in `requestEncryption` | `g_b = g^b mod p`, sent in `acceptEncryption` |
| validate peer value | check `g_b` per §1.2 | check `g_a` per §1.2 |
| shared key | `key = g_b^a mod p`, pad to 256 B | `key = g_a^b mod p`, pad to 256 B |
| fingerprint | **compare** own against `encryptedChat.key_fingerprint` | **compute** and **send** it |
| `x` for crypto | `x = 0` outgoing, `x = 8` incoming | `x = 8` outgoing, `x = 0` incoming |
| `seq_no` parity | `out` odd, `in` even | `out` even, `in` odd |

The `x` and parity rows are derived in §2.5 and §3.4. They are listed here because they are the only
per-side asymmetry in the whole protocol, and getting the side wrong is the single most likely
silent failure (Constitution Principle I).

### §1.7 Authorization binding and the update queue

`[DOC:api/end-to-end]`: "Secret chats are associated with specific devices (or rather with
authorization keys), not users." Updates arrive through a separate `qts` queue, acknowledged with
`messages.receivedQueue` or implicitly by `updates.getDifference`. "All messages acknowledged as
delivered by the client, as well as any messages older than 7 days, may (and will) be deleted from
the server." On de-authorization the queue is cleared and `qts` becomes irrelevant.

Consequence for this package: a secret chat cannot be exported to another session, and the update
pipeline must not drop `UpdateNewEncryptedMessage` silently — a dropped update becomes a permanent
`out_seq_no` gap (§3.5).

---

## §2. Key derivation, MTProto 2.0

All of §2 is `[DOC:api/end-to-end]` unless marked otherwise.

### §2.1 Serialization and length prefix

"A TL object of type DecryptedMessage is created ... the object must be wrapped in the constructor
decryptedMessageLayer ... The resulting construct is serialized as an array of bytes using generic
TL rules. The resulting array is prepended by 4 bytes containing the array length not counting these
4 bytes."

The plaintext buffer is therefore `len:uint32_le || TL-serialized decryptedMessageLayer`.

### §2.2 Padding

"The byte array is padded with 12 to 1024 random padding bytes to make its length divisible by 16
bytes. (In the older MTProto 1.0 encryption, only 0 to 15 padding bytes were used.)"

Two constraints, both mandatory:

1. padding length in `[12, 1024]`;
2. `(4 + payload + padding) % 16 == 0`.

The smallest conforming choice is `12 + ((16 - (4 + payload + 12) % 16) % 16)`, i.e. 12..27 bytes.

`[TD:td/mtproto/Transport.cpp:165-183]` TDLib does not take the minimum. `do_calc_crypto_size2_basic`
applies `(enc_size + data_size + 12 + 15) & ~15` and then rounds up to the next of
`{64, 128, 192, 256, 384, 512, 768, 1024, 1280}`, and beyond 1280 to 448-byte steps — the `>= 12`
floor and the 16-alignment hold, but the padding routinely exceeds 27 bytes and the total can exceed
the documented 1024 for large messages. `do_calc_crypto_size2_rand` (line 180) instead adds
`Random::secure_uint32() & 0xff` bytes on top.

**UNVERIFIED:** no consulted source says a receiver must reject padding longer than 1024 bytes, and
the oracle itself can produce more. Treat 1024 as a sender-side guideline; do not add a receive-side
upper bound without an interop test proving official clients honour it.

### §2.3 `msg_key`

Verbatim from `[DOC:api/end-to-end]`:

```
msg_key_large = SHA256 (substr (key, 88+x, 32) + plaintext + random_padding);
msg_key       = substr (msg_key_large, 8, 16);
```

The prose restates it: "Message key, msg_key, is computed as the 128 middle bits of the SHA256 of
the data obtained in the previous step, prepended by 32 bytes from the shared key key."

Two properties distinguish this from MTProto 1.0 and must not be lost: the padding **is** inside the
hash, and 32 bytes of the shared key **are** inside the hash.

`[TD:td/mtproto/Transport.cpp:148-164]` `Transport::calc_message_key2` carries the doc line as a
comment, feeds `Slice(auth_key.key()).substr(88 + X, 32)` and then the whole `to_encrypt` slice
(plaintext **and** padding) into one SHA-256, and returns `msg_key_large.substr(8, 16)`.

### §2.4 `aes_key` and `aes_iv`

Verbatim from `[DOC:api/end-to-end]`:

```
sha256_a = SHA256 (msg_key + substr (key, x, 36));
sha256_b = SHA256 (substr (key, 40+x, 36) + msg_key);
aes_key  = substr (sha256_a, 0, 8) + substr (sha256_b, 8, 16) + substr (sha256_a, 24, 8);
aes_iv   = substr (sha256_b, 0, 8) + substr (sha256_a, 8, 16) + substr (sha256_b, 24, 8);
```

`aes_key` and `aes_iv` are mirror images, taking `a`/`b` in opposite order; both are 32 bytes.
Encryption is "AES-256 encryption with infinite garble extension (IGE)".

`[TD:td/mtproto/Transport.cpp:238, 362]` calls
`KDF2(auth_key.key(), header->message_key, X, &aes_key, &aes_iv)` on the read and write paths;
MTProto 1.0 uses the separate `KDF` at lines 236 and 359.

### §2.5 The `x = 0 / x = 8` sender distinction

`[DOC:api/end-to-end]`: "For MTProto 2.0, x=0 for messages from the originator of the secret chat,
x=8 for the messages in the opposite direction."

"Originator" is the side that called `messages.requestEncryption` (A in §1.3).

`[TD:td/mtproto/Transport.cpp:405]` write path chooses
`packet_info->is_creator || packet_info->version == 1 ? 0 : 8`.
`[TD:td/mtproto/Transport.cpp:321]` read path chooses
`packet_info->is_creator && packet_info->version != 1 ? 8 : 0`.
`[TD:td/telegram/SecretChatActor.cpp:223]` sets `packet_info.is_creator = auth_state_.x == 0`.

Read together: **I am the originator ⇒ my outgoing messages use `x = 0` and the messages I receive
use `x = 8`.** The peer sees the mirror image. Both `x` values index the same 256-byte shared key, so
the two directions never share an `aes_key`.

### §2.6 Outer frame

"Encryption key fingerprint key_fingerprint and the message key msg_key are added at the top of the
resulting byte array."

```
key_fingerprint : 8 bytes   (§1.5; little-endian int64 of the last 8 bytes of SHA1(key))
msg_key         : 16 bytes  (§2.3)
ciphertext      : n bytes   (AES-256-IGE, n % 16 == 0)
```

The result goes into `messages.sendEncrypted`, `messages.sendEncryptedService`, or
`messages.sendEncryptedFile`.

`key_fingerprint` is what lets a receiver pick between the current key and the previous key during a
rekey (§4.5): `[TD:td/telegram/SecretChatActor.cpp:771-773]` compares the incoming `auth_key_id`
against `pfs_state_.auth_key.id()` and then `pfs_state_.other_auth_key.id()`.

### §2.7 Receive-side checks

`[DOC:api/end-to-end]`: "When an encrypted message is received, you must check that msg_key is in
fact equal to the 128 middle bits of the SHA256 hash of the decrypted message, prepended by 32 bytes
taken from the shared key."

A conforming receiver verifies, in order:

1. `key_fingerprint` matches a key it holds (current or, during rekey, previous);
2. after IGE decryption, the recomputed `msg_key` equals the received `msg_key` — the integrity check;
3. the 4-byte length prefix does not exceed the decrypted buffer;
4. remaining padding `>= 12` (§2.2);
5. total decrypted length is a multiple of 16;
6. the layer and `seq_no` checks of §3.

Constitution "Secret Material Handling": a message failing any of these MUST be rejected, not
repaired. Checks 2 and 3 in particular are the difference between a decryption failure and a
buffer-length oracle.

---

## §3. Message framing

### §3.1 `decryptedMessageLayer`

```
decryptedMessageLayer#1be31789 random_bytes:bytes layer:int in_seq_no:int out_seq_no:int
                               message:DecryptedMessage = DecryptedMessageLayer;
```
`[DOC:schema/end-to-end]`

Every message, service messages included, is wrapped in it. `[DOC:api/end-to-end/seq_no]`: "All
Secret Chats messages in clients using Layer 17 or higher are wrapped in decryptedMessageLayer and
have seq_no (sequence number) counters attached to them. Note that any service messages in secret
chats must also increment the seq_no."

Both `DecryptedMessage` service constructors appear on the wire:

```
decryptedMessageService#73164160 random_id:long action:DecryptedMessageAction = DecryptedMessage;
decryptedMessageService#aa48327d random_id:long random_bytes:bytes action:DecryptedMessageAction = DecryptedMessage;
```

The `aa48327d` form is the obsolete layer-8 shape. `[TD:td/telegram/SecretChatActor.cpp:939-943]` and
`[TD:td/telegram/SecretChatActor.cpp:1231-1235]` normalise an inbound `decryptedMessageService8` into
`decryptedMessageService` before dispatch; a receiver must accept both.

### §3.2 `layer_no`

`[DOC:api/end-to-end]`: "the object must be wrapped in the constructor decryptedMessageLayer with an
indication of the supported layer (starting with 46)." Handling of the value is §7.

### §3.3 `random_bytes`

The published schema types the field as `bytes` and no consulted page states a minimum length.

`[TD:td/telegram/SecretChatActor.cpp:215-216]` TDLib always sends **exactly 31 bytes**:
`BufferSlice random_bytes(31); Random::secure_bytes(random_bytes.as_mutable_slice());`

**UNVERIFIED:** whether any minimum is enforced by peers or by the server. 31 bytes is the oracle's
behaviour and is the safe choice; a shorter value is neither documented as rejected nor documented as
acceptable. Do not derive a rule from the base package here (§8.3).

### §3.4 `in_seq_no` / `out_seq_no` — the transform

`[DOC:api/end-to-end/seq_no]`: "The seq_no counters in their raw form are initialized with
(out_seq_no, in_seq_no) := (0,0), and incremented strictly by 1 after any message (service or not)
is sent/received and processed. They must be protected from mirroring before being sent to the
remote client by transformation according to formula 2*raw_seq_no+x, where x is 0 or 1".

| | `in_seq_no` `x` | `out_seq_no` `x` |
|---|---|---|
| secret chat initiated by sender | 0 | 1 |
| secret chat initiated by recipient | 1 | 0 |

"In this way the least significant bit of each seq_no field included in the message is different for
incoming and outgoing messages. This is done to prevent a possible attacker from mirroring the
messages. **If any of the received in_seq_no or out_seq_no are not consistent in terms of parity (see
table above), the client is required to immediately abort the secret chat.**"

`[TD:td/telegram/SecretChatActor.cpp:884-886]`:
`if (in_seq_no % 2 != (1 - auth_state_.x) || out_seq_no % 2 != auth_state_.x) return Status::Error("Bad seq_no parity");`
— the same `auth_state_.x` that selects the crypto `x` in §2.5 (0 for the originator), so one stored
flag drives both.

`[DOC:api/end-to-end/seq_no]`: "assign in_seq_no and out_seq_no to each message at the exact moment
when the message is created, and never change them in the future"; and outgoing messages must be
placed in an `invokeAfterMsgs` chain so the server queues them in order, because "Failure to do this
may result in gaps on the remote client, which may in turn lead to aborted secret chats."

### §3.5 Validating `out_seq_no` (the peer's counter)

`[DOC:api/end-to-end/seq_no]`: "Your client must check that it has received each message with the
sequence number out_seq_no starting from 0 to some current point C. It should then expect the next
message to have the sequence number out_seq_no=C+1."

- `out_seq_no <= C` — "the local client must drop the message (repeated message). The client should
  not check the contents of the message because the original message could have been deleted".
- `out_seq_no > C+1` — a gap. "A temporary solution to this is to simply abort the secret chat. But
  since this may cause some existing older secret chats to be aborted, it is strongly recommended for
  the client to properly handle such seq_no gaps. Note that in_seq_no is not increased upon receipt
  of such a message; it is advanced only after all preceding gaps are filled."

`[TD:td/telegram/SecretChatActor.cpp:889-894]` returns `Status::Error(1, "Old seq_no")` for the first
case and `Status::Error(2, "Gap found!")` for the second; code 2 is the only status that does **not**
discard the message (`[TD:td/telegram/SecretChatActor.cpp:913]`
`if (status.is_error() && status.code() != 2 /* not gap found */)`) — gaps queue, everything else
drops.

### §3.6 Validating `in_seq_no` (my counter, echoed back)

`[DOC:api/end-to-end/seq_no]`, both conditions mandatory:

1. "in_seq_no must form a non-decreasing sequence of non-negative integer numbers."
2. "if D is the out_seq_no of last message we sent, the received in_seq_no should not be greater than
   D + 1."

The second also orders the conversation: the worked example in the source is a reply with
`in_seq_no=2` arriving after five sent messages — "the local client must place that message after the
second message it sent. This makes manipulations with delayed messages impossible."

"If in_seq_no contradicts these criteria, the local client is required to immediately abort the
secret chat."

`[TD:td/telegram/SecretChatActor.cpp:895-903]` implements both, as `"in_seq_no is not monotonic"` and
`"in_seq_no is bigger than seq_no_state_.my_out_seq_no"`, and adds a third check with no direct
documentation counterpart:
`if (his_layer < seq_no_state_.his_layer) return Status::Error("his_layer is not monotonic");` — a
peer may not walk its announced layer backwards. **UNVERIFIED** as a documented requirement; it is
oracle behaviour, which under Constitution Principle II makes it the tie-breaker.

### §3.7 Gap handling and `decryptedMessageActionResend`

`[DOC:api/end-to-end/seq_no]`: put out-of-order messages "into a 'waiting queue' on the local client,
and re-request the missing messages using the special constructor decryptedMessageActionResend".

The bounds are sent already transformed (§3.4), not raw: "you can easily get the necessary
start_seq_no by adding 2 to the out_seq_no of the last message before the hole and the end_seq_no by
subtracting 2 from the out_seq_no of the received message with the wrong sequence number."

`[TD:td/telegram/SecretChatActor.cpp:566-570]` composes it as `start_seq_no * 2 + auth_state_.x` /
`finish_seq_no * 2 + auth_state_.x`; `[TD:td/telegram/SecretChatActor.cpp:952-953]` divides by 2 on
receipt.

Rules that shape the state machine:

- One request per hole. "if the remote client keeps sending out of sync messages, they should be put
  into the queue without sending a new request."
- Interpret the recovered messages in `seq_no` order first, then drain the queue in `seq_no` order.
- "having two gaps simultaneously is very rare ... and it is acceptable to abort the secret chat in
  this situation."
- "If a local client receives decryptedMessageActionResend but is unable to satisfy the request, it
  must abort the secret chat."
- **Resend is the one exception to in-order interpretation**: "decryptedMessageActionResend must
  always be interpreted immediately upon receipt in all cases, even if its out_seq_no>=C+1. Note that
  each decryptedMessageActionResend must only be handled once, it must not be interpreted again when
  we interpret messages in the queue."

`[TD:td/telegram/SecretChatActor.h:140]` caps the span: `static constexpr int32 MAX_RESEND_COUNT = 1000;`
and `[TD:td/telegram/SecretChatActor.cpp:955-957]` rejects a wider request with
`"Can't resend too many messages"`. After servicing it, TDLib rewrites the action to `Noop` in place
(`[TD:td/telegram/SecretChatActor.cpp:968]`) so a binlog replay cannot resend twice — that is the
"handled once" rule made durable.

### §3.8 Deleting an unacknowledged message

`[DOC:api/end-to-end/seq_no]` — required, because a deleted message must not leave a hole:

1. "securely destroy the contents of the message";
2. "change the local copy of the original message to decryptedMessageActionDeleteMessages with
   random_id equal to its own random_id";
3. "create a new outgoing message deleting the original message."

"otherwise it must arrive as a 'self-delete' message to maintain the correct sequence of seq_no."

---

## §4. Rekeying / Perfect Forward Secrecy

All of §4 is `[DOC:api/end-to-end/pfs]` unless marked otherwise. PFS exists as of Layer 20. "Please
note that your client must support Forward Secrecy in Secret Chats to be compatible with official
Telegram clients."

### §4.1 Trigger

"official Telegram clients will initiate re-keying once a key has been used to decrypt and encrypt
more than 100 messages, or has been in use for more than one week, provided the key has been used to
encrypt at least one message."

`[TD:td/telegram/SecretChatActor.cpp:576-580]`:
```
if (pfs_state_.state == PfsState::Empty &&
    (pfs_state_.last_message_id + 100 < seq_no_state_.message_id ||
     pfs_state_.last_timestamp + 60 * 60 * 24 * 7 < Time::now()) &&
    pfs_state_.other_auth_key.empty()) { request_new_key(); }
```
100 messages counted on the shared message counter, or 7 days wall-clock — and only when no exchange
is running and no previous key is still being retained.

"you should never initiate a new instance of the re-keying protocol if an uncompleted instance
exists, initiated by either party."

### §4.2 `decryptedMessageActionRequestKey` — A

```
decryptedMessageActionRequestKey#f3c9611b exchange_id:long g_a:bytes = DecryptedMessageAction;
```

"exchange_id is a random number identifying this instance of the Re-Keying Protocol for both
parties". "Note that the same Diffie--Hellman parameters (p,g) as for the initial Diffie--Hellman key
exchange in this secret chat are used. They do not need to be re-transmitted explicitly."

`a` is subject to "the same limitations as for the initial Diffie-Hellman key exchange" — the §1.2
checks apply in full to the rekey values.

### §4.3 `decryptedMessageActionAcceptKey` — B

```
decryptedMessageActionAcceptKey#6fe1735b exchange_id:long g_b:bytes key_fingerprint:long = DecryptedMessageAction;
```

"key_fingerprint is the 64-bit fingerprint of the newly generated key = pow(g_a, b) mod p, used as a
sanity check of the implementation" — the same SHA-1 rule as §1.5.

"At this stage, B can already compute the new key ... However, it continues using the previous key
until the completion of the exchange."

**Point of no return:** "Once side B sends decryptedMessageActionAcceptKey, it cannot abort the key
exchange; it must be ready to switch to the new key immediately after a
decryptedMessageActionCommitKey is received. Therefore, if side B wishes to delay the usage of new
key, for example in order to fill some seq_no gaps first, it must delay the
decryptedMessageActionAcceptKey answer accordingly."

### §4.4 `decryptedMessageActionCommitKey` — A

```
decryptedMessageActionCommitKey#ec2e0b9b exchange_id:long key_fingerprint:long = DecryptedMessageAction;
```

"Once A receives a valid decryptedMessageActionAcceptKey, it performs all necessary checks, and
'commits' the new key ... After that, A can (and must) encrypt all following messages with the new
key." A may delay the commit to fill its own gaps first.

### §4.5 Final step and `Noop`

"When B receives either a decryptedMessageActionCommitKey **or a message encrypted by the new key,
recognized by the value of key_fingerprint prepended to the encrypted message** (it may happen that
the decryptedMessageActionCommitKey has been lost and will be re-requested later), it assumes that A
has started using the new key for encryption, and does the same."

A cannot discard the old key on its own: "A may only discard the previous key after a message
encrypted with the new key has been received. If no ordinary messages are scheduled to be sent, a
special no-op message should sent by B for this purpose:"

```
decryptedMessageActionNoop#a82fdd63 = DecryptedMessageAction;
```

### §4.6 `decryptedMessageActionAbortKey`

```
decryptedMessageActionAbortKey#dd05ec6b exchange_id:long = DecryptedMessageAction;
```

"Any of the parties may abort any instance of an uncompleted re-keying protocol, **unless
decryptedMessageActionCommitKey or decryptedMessageActionAcceptKey has been already sent by the party
in question**." Grounds: already participating in another instance, or "the received values of g_a,
g_b and other parameters do not pass security checks. In the latter case, it might be advisable to
abort the Secret Chat altogether."

### §4.7 Concurrent rekeying

Both sides may send `RequestKey` at once. If each aborted because it was already in an exchange, "the
re-keying will never happen." The documented tie-break, comparing `exchange_id` "as a long, i.e.
signed little-endian 64-bit integer":

- mine **larger** than the received one — silently abandon the newly-suggested instance, send no
  `AbortKey`;
- mine **smaller** — answer the received `RequestKey` with `AcceptKey` and participate only in the
  peer's instance; my `(a, g_a)` may be reused as `(b, g_b)` or regenerated;
- equal (probability 2^-64) — "abort both instances without sending an explicit
  decryptedMessageActionAbortKey. The other side will do the same."

### §4.8 In-flight messages during the exchange

- Both sides keep encrypting with the **old** key until the commit point of their own role (§4.3,
  §4.4). No message is re-encrypted or re-sent because of a rekey.
- After the switch, the previous key "may be kept until there are no gaps in received messages up to
  the switch to the new key. Once all the gaps have been filled, the old key must be securely
  discarded." A receiver therefore holds up to two keys and selects by the `key_fingerprint` prefix
  (§2.6); `[TD:td/telegram/SecretChatActor.cpp:771-773]` is that selection and
  `[TD:td/telegram/SecretChatActor.cpp:1213-1216]` the drop of `other_auth_key`.
- `seq_no` counters are **not** reset by a rekey: nothing in `[DOC:api/end-to-end/pfs]` or
  `[DOC:api/end-to-end/seq_no]` resets them, and TDLib's `seq_no_state_` is independent of
  `pfs_state_`.

### §4.9 Key visualization

Not part of the exchange, but it constrains what may be discarded: "the SHA-1 of the original key
(generated during the establishment of Secret Chat in question) is always stored, in order to show
key visualizations." The visualization is "the first 128-bits of the SHA-1 of the original key ...
followed by the first 160 bits of the SHA-256 of the key in use when the secret chat was updated to
layer 46". `key_fingerprint` "was introduced as a maintenance tool (with a misleading name) and is
not related to key visualization".

---

## §5. Service actions

The complete `DecryptedMessageAction` set from `[DOC:schema/end-to-end]` (Layer 223) — thirteen
constructors, no more. Each is carried inside a `decryptedMessageService` (§3.1) and, like any
message, increments `seq_no` (§3.4).

| # | Constructor | Meaning / required handling |
|---|---|---|
| 5.1 | `decryptedMessageActionSetMessageTTL#a1733aec ttl_seconds:int` | Peer set the self-destruct timer for the chat. Store it and apply it to subsequent messages. `ttl_seconds = 0` disables. |
| 5.2 | `decryptedMessageActionReadMessages#c4f40be random_ids:Vector<long>` | Peer read those messages; for TTL messages this is what starts the countdown on the sender's copy. |
| 5.3 | `decryptedMessageActionDeleteMessages#65614304 random_ids:Vector<long>` | Delete those messages locally. Also the "self-delete" vehicle of §3.8. |
| 5.4 | `decryptedMessageActionScreenshotMessages#8ac1f475 random_ids:Vector<long>` | Peer reports taking a screenshot of those messages; surface it. |
| 5.5 | `decryptedMessageActionFlushHistory#6719e45c` | Clear the entire chat history locally. |
| 5.6 | `decryptedMessageActionResend#511110b0 start_seq_no:int end_seq_no:int` | Re-request / re-send a range. Full rules in §3.7 — transformed seq_no, handled immediately and exactly once, abort the chat if unsatisfiable. |
| 5.7 | `decryptedMessageActionNotifyLayer#f3048883 layer:int` | Peer announces its layer. Full rules in §7. |
| 5.8 | `decryptedMessageActionTyping#ccb27641 action:SendMessageAction` | Typing / recording indicator, reusing the cloud-chat `SendMessageAction` type. |
| 5.9 | `decryptedMessageActionRequestKey#f3c9611b exchange_id:long g_a:bytes` | Rekey step 1 — §4.2. |
| 5.10 | `decryptedMessageActionAcceptKey#6fe1735b exchange_id:long g_b:bytes key_fingerprint:long` | Rekey step 2 — §4.3. |
| 5.11 | `decryptedMessageActionCommitKey#ec2e0b9b exchange_id:long key_fingerprint:long` | Rekey step 3 — §4.4. |
| 5.12 | `decryptedMessageActionAbortKey#dd05ec6b exchange_id:long` | Rekey aborted — §4.6. Receiving it must clear the local exchange state. |
| 5.13 | `decryptedMessageActionNoop#a82fdd63` | Empty message whose only purpose is to let the peer retire the old key — §4.5. |

A usable client must **handle** all thirteen on receipt. It must be able to **send** at least 5.1,
5.2, 5.3, 5.4, 5.6, 5.7, and 5.9–5.13; 5.5 and 5.8 are optional outbound features.

The consuming MCP server exposes tools over 5.1 `SetMessageTTL`, 5.2 `ReadMessages`, 5.3
`DeleteMessages`, 5.4 `ScreenshotMessages`, 5.6 `Resend` and 5.7 `NotifyLayer`, so all six need a
public send path, not only an inbound branch.

**UNVERIFIED:** the precise moment a TTL countdown starts for each media type, and whether
`ScreenshotMessages` has any required side effect beyond notification. Neither is stated on the
consulted pages.

---

## §6. Files

### §6.1 One-time keys, unrelated to the chat key

`[DOC:api/end-to-end]`: "All files sent to secret chats are encrypted with one-time keys that are in
no way related to the chat's shared key."

"Prior to a file being sent to a secret chat, 2 random 256-bit numbers are computed which will serve
as the AES key and initialization vector used to encrypt the file. AES-256 encryption with infinite
garble extension (IGE) is used in like manner."

So per file: `key` 32 bytes, `iv` 32 bytes, both fresh from a CSPRNG, IGE like §2.4 but with no `x`,
no `msg_key`, and no relation to `key` of §1.4.

### §6.2 `key_fingerprint` for files — a different algorithm from §1.5

`[DOC:api/end-to-end]`:

```
digest      = md5(key + iv)
fingerprint = substr(digest, 0, 4) XOR substr(digest, 4, 4)
```

MD5, not SHA-1; 32 bits, not 64. It is transmitted **outside** the encrypted message, in the
`encryptedFile` the server returns, so the receiver can check that the `key`/`iv` it found inside the
decrypted message belong to the attached file.

### §6.3 Where the key travels versus where the address travels

`[DOC:api/end-to-end]`: "it is assumed that the encrypted file's address will be attached to the
outside of an encrypted message using the file parameter of the messages.sendEncryptedFile method and
that the key for direct decryption will be sent in the body of the message (the key parameter
contained in DecryptedMessageMedia constructors)."

- Outside, in cleartext to the server: file id, access hash, size, dc, and the 32-bit fingerprint.
- Inside, end-to-end encrypted: `key`, `iv`, and the media metadata (`mime_type`, `size`, `thumb`,
  attributes).

```
encryptedFile#a8008cd8 id:long access_hash:long size:long dc_id:int key_fingerprint:int = EncryptedFile;
```
`[DOC:constructor/encryptedFile]` — note `size:long`, the big-file form; the `size:int` predecessor
corresponds to pre-143 layers (§7.1).

### §6.4 Upload and download versus ordinary files

`[DOC:api/end-to-end]`: "The encrypted contents of a file are stored on the server in much the same
way as those of a file in cloud chats: piece by piece using calls to upload.saveFilePart. A
subsequent call to messages.sendEncryptedFile will assign an identifier to the stored file and send
the address together with the message."

Differences from an ordinary file:

1. the bytes are IGE-encrypted client-side **before** `upload.saveFilePart`, so the server stores
   ciphertext only;
2. the send call is `messages.sendEncryptedFile`, taking `InputEncryptedFile`
   (`inputEncryptedFileUploaded` / `inputEncryptedFileBigUploaded`) rather than `InputMedia`;
3. download uses `inputEncryptedFileLocation` and the caller decrypts with the `key`/`iv` from the
   message body;
4. forwarding does not re-upload: "Incoming and outgoing encrypted files can be forwarded to other
   secret chats using the constructor inputEncryptedFile to avoid saving the same content on the
   server twice." Note the forwarded copy keeps its original `key`/`iv`.

A receiver MUST recompute §6.2 from the `key`/`iv` inside the message and compare it against
`encryptedFile.key_fingerprint` before decrypting; a mismatch is a rejection, not a warning.

---

## §7. Layer and compatibility

### §7.1 The layer numbers that matter

| Layer | Why it matters | Source |
|---|---|---|
| 8 | Obsolete. `decryptedMessageService#aa48327d` still arrives in this shape (§3.1). Out of scope for this project per Constitution Principle II. | `[DOC:api/end-to-end]` |
| 17 | First layer with `decryptedMessageLayer` and `seq_no`. | `[DOC:api/end-to-end/seq_no]` |
| 20 | PFS / rekeying introduced. | `[DOC:api/end-to-end/pfs]` |
| **46** | Initial assumed remote layer, and the minimum that may appear in `decryptedMessageLayer.layer`. | `[DOC:api/end-to-end]` |
| **73** | MTProto 2.0 becomes mandatory both ways. TDLib's `Default` **and** `Mtproto2`. | `[DOC:api/end-to-end]`, `[TD:td/telegram/SecretChatLayer.h:12-13]` |
| 101 | New message entities. | `[TD:td/telegram/SecretChatLayer.h:14]` |
| 123 | Delete messages on close. | `[TD:td/telegram/SecretChatLayer.h:15]` |
| 143 | Big file support (`encryptedFile.size:long`, §6.3). | `[TD:td/telegram/SecretChatLayer.h:16]` |
| **144** | Spoiler and custom-emoji entities; TDLib's `Current`. | `[TD:td/telegram/SecretChatLayer.h:17-18]` |

The E2E schema page itself is published under "Layer 223", which is the **API** layer of the page,
not the secret-chat layer. The secret-chat layer ceiling in the oracle is 144.

### §7.2 Tracking the remote layer

`[DOC:api/end-to-end]`: "Your client should always store the maximal layer that is known to be
supported by the client on the other side of a secret chat. When the secret chat is first created,
this value should be initialized to 46. This remote layer value must always be updated immediately
after receiving any packet containing information of an upper layer, i.e.:

- any secret chat message containing layer_no in its decryptedMessageLayer with layer>=46, or
- a decryptedMessageActionNotifyLayer service message, wrapped as if it were the
  decryptedMessageService constructor of the obsolete layer 8 (constructor
  decryptedMessageService#aa48327d)."

`[TD:td/telegram/SecretChatActor.cpp:852-856]` raises `config_state_.his_layer` and persists it
whenever a higher layer arrives. It never lowers it, and §3.6 rejects a message whose layer is below
the stored one.

### §7.3 Announcing the local layer

`[DOC:api/end-to-end]`: "your client must send a message of the decryptedMessageActionNotifyLayer
type. This notification must be wrapped in a constructor of an appropriate layer." Two mandatory
occasions:

1. "As soon as a new secret chat has been created, immediately after the secret key has been
   successfully exchanged."
2. "Immediately after the local client has been updated to support a new secret chat layer. In this
   case notifications must be sent to all currently existing secret chats." Only needed when the new
   layer actually changes secret-chat behaviour.

`[TD:td/telegram/SecretChatActor.cpp:872-876]` also sends `NotifyLayer` as a **recovery** action when
an inbound packet fails to parse ("support for older layer"), which is not in the documentation.

### §7.4 The layer a client must use for outgoing messages

`[TD:td/telegram/SecretChatActor.h:649-658]` `current_layer()`:

```
layer = SecretChatLayer::Current;                       // 144
if (config_state_.his_layer < layer) layer = his_layer;  // never above the peer
if (layer < SecretChatLayer::Default) layer = Default;   // never below 73
```

So the announced/used layer is `clamp(his_layer, 73, 144)`.

### §7.5 What "both parties at 73+" changes, and what to do below it

`[DOC:api/end-to-end]`: "As soon as both parties in a secret chat are using at least Layer 73, they
should only use MTProto 2.0 for all outgoing messages. Some of the first received messages may use
MTProto 1.0, if a sufficiently high starting layer has not been negotiated during the creation of the
secret chat. After the first message encrypted with MTProto 2.0 (or the first message with Layer 73
or higher) is received, all messages with higher sequence numbers must be encrypted with MTProto 2.0
as well."

And for the transitional receive path: "As long as the current layer is lower than 73, each party
should try to decrypt received messages with MTProto 1.0, and if this is not successful (msg_key does
not match), try MTProto 2.0. Once the first MTProto 2.0-encrypted message arrives (or the layer is
upgraded to 73), there is no need to try MTProto 1.0 decryption for any of the further messages
(unless the client is still waiting for some gaps to be closed)."

The oracle is stricter than the prose in both directions and these two facts settle the design:

- **Outgoing is always MTProto 2.0.** `[TD:td/telegram/SecretChatActor.cpp:222]`
  `packet_info.version = 2;` is unconditional in `create_encrypted_message`. There is no downgrade
  path on send.
- **Incoming tries 2 then 1, and a claimed layer forbids the downgrade.**
  `[TD:td/telegram/SecretChatActor.cpp:783-801]` iterates `versions{{2, 1}}`, and
  `[TD:td/telegram/SecretChatActor.cpp:857-859]`:
  `if (layer >= SecretChatLayer::Mtproto2 && mtproto_version < 2) return Status::Error("MTProto 1.0 encryption is forbidden for this layer");`
  — a peer announcing 73+ inside a message that was decrypted as MTProto 1.0 is rejected, which closes
  the obvious downgrade.

**What a client MUST do when the peer announces a lower layer:** clamp its own outgoing layer to the
peer's (`§7.4`) so the peer can parse it, never lower its own floor below 73, never lower
`his_layer` once raised (§7.2), and never downgrade its own encryption — a peer that has ever claimed
73+ may not be served MTProto 1.0, and this project does not implement MTProto 1.0 at all
(Constitution Principle II).

`[DOC:api/end-to-end]` also requires the opposite direction: "If the message layer is greater than
the one supported by the client, the user must be notified that the client version is out of date and
prompted to update."

---

## §8. Gap analysis against `painor/telethon-secret-chat`

Source read: `https://github.com/painor/telethon-secret-chat/archive/refs/heads/master.zip`,
unpacked read-only into a scratch directory, 2026-09-19. Version `0.2.4`, MIT, **no tests, no CI, no
`LICENSE` header in the sources**.

### §8.0 Confirmed measurements

The coordinator's figures reproduce exactly:

| File | Lines |
|---|---|
| `secret_sechma/secretTL.py` (generated) | 2294 |
| `secret_methods.py` | 664 |
| `storage/sqlite.py` + `abstract.py` + `memory.py` + `__init__.py` | 134 + 85 + 56 + 0 = 275 |
| `secret_chat_manager.py` | 95 |
| `secret_sechma/__init__.py` | 84 |
| `setup.py` | 36 |
| `__init__.py` + `version.py` | 7 + 3 |
| **total** | **3458** |

**One correction.** "Exactly two Telethon internals" undercounts. There are **four** private
attributes plus one mutated global:

1. `client._log` — `secret_chat_manager.py:43`.
2. `client._parse_message_text` — `secret_methods.py:440`.
3. `MTProtoState._calc_key` — `secret_methods.py:405, 583`. A private static method on
   `telethon.network.mtprotostate.MTProtoState`, and it is **the entire MTProto 2.0 KDF** (§2.4). This
   is the highest-risk dependency in the package: if Telethon renames or changes it, every secret chat
   breaks, and the failure is a decryption error rather than an import error only because
   `handle_encrypted_update` swallows it into the MTProto 1.0 fallback (§8.2).
4. `SQLiteSession._conn` — `secret_chat_manager.py:39`.
5. `telethon.tl.alltlobjects.tlobjects` is mutated process-wide by `patch_tlobjects()`
   (`secret_chat_manager.py:21-22`), so the E2E constructors are registered into Telethon's global
   registry. Not underscore-private, but a global side effect with the same compatibility risk.

Constitution "Development Workflow" requires all of these in one list with a test that fails when any
disappears.

### §8.1 Chat establishment (§1)

| Item | State | Evidence |
|---|---|---|
| `getDhConfig` + version caching | **implemented** | `get_dh_config()`, `secret_methods.py:144` |
| `p` is a safe 2048-bit prime | **absent** | `get_dh_config` only does `int.from_bytes(dh_config.p, 'big')`; there is no primality, bit-length or `(p-1)/2` test anywhere in the package |
| `g` quadratic-residue condition | **absent** | no `p mod 8` / `mod 3` / `mod 5` / `mod 24` / `mod 7` test exists |
| `1 < g_a < p-1` and the `2^1984` bound | **implemented** | `check_g_a()`, `secret_methods.py:155-160`, applied to `g_a`, `g_b` and `g_a_or_b` |
| `1 < g < p-1` | **absent** | `g` itself is never range-checked |
| `requestEncryption` | **implemented** | `start_secret_chat()`, `:162` |
| `acceptEncryption` | **implemented** | `accept_secret_chat()`, `:624` |
| `key_fingerprint` compute & compare | **implemented** | `:643` (B computes), `:658-660` (A compares, raises `ValueError`) |
| 256-byte left-padding of the shared key | **implemented** | `.to_bytes(256, 'big')` throughout |

### §8.2 Key derivation (§2)

| Item | State | Evidence |
|---|---|---|
| `msg_key` = `SHA256(key[88+x:120+x] + plaintext + padding)[8:24]` | **implemented** | `encrypt_secret_message`, `:403-404` |
| `x` selection | **implemented and correct** | encrypt `0 if peer.admin else 8` (`:402`), decrypt `8 if peer.admin else 0` (`:590`); `admin=True` is set only for the requester (`finish_secret_chat_creation`, `:663`) |
| `aes_key` / `aes_iv` KDF2 | **delegated** to `MTProtoState._calc_key` | `:405, 583` — correct-looking but **never proved**; Constitution Principle I makes this the first thing an oracle fixture must pin |
| padding `>= 12`, 16-aligned | **implemented** | `:399-401`, yields 12..27 bytes |
| `key_fingerprint` outer prefix | **implemented** | `:412-413` |
| incoming `auth_key_id` check | **implemented but broken on the failure path** | `:347-350` calls `self.close_secret_chat(message.chat_id)` with an **int**, and `close_secret_chat` immediately does `peer.id` (`:571`) — so the security-check branch raises `AttributeError` instead of discarding the chat |
| `msg_key` re-verification on decrypt | **implemented** | `:592-593` |
| padding-length and 16-alignment checks on decrypt | **implemented** | `:594-597` |
| length-prefix bound check | **partial** | `:588` tests `message_data_length > len(decrypted_data)`, not `> len(decrypted_data) - 4` |
| **MTProto 1.0 never used** (§7.5) | **violated** | `SecretChat.__init__` defaults `mtproto=1` (`:57`), and `encrypt_secret_message` branches on it (`:400`), so **the first outgoing messages of every chat are MTProto 1.0** until the peer happens to announce layer >= 73. TDLib sends v2 unconditionally (`[TD:td/telegram/SecretChatActor.cpp:222]`) |
| downgrade resistance | **violated** | `:357-362` catches **any** exception from `decrypt_mtproto2`, retries MTProto 1.0 and **permanently sets `peer.mtproto = 1`**. There is no equivalent of TDLib's "MTProto 1.0 encryption is forbidden for this layer" |

Also: `_old_calc_key(..., True)` is called with a hard-coded `client=True` on both decrypt paths
(`:606-608`), so the MTProto 1.0 `x` is always 0 regardless of side — wrong, though moot once
MTProto 1.0 is removed.

### §8.3 Message framing (§3) — the weakest area

| Item | State | Evidence |
|---|---|---|
| `decryptedMessageLayer` wrapping | **implemented** | `:394-398` |
| `random_bytes` | **partial** | `os.urandom(15 + 4 * random.randint(0, 2))` → 15/19/23 bytes (`:396`, and `:563`). Uses the non-cryptographic `random` module to choose the length, which Constitution "Secret Material Handling" forbids in any path producing padding |
| `2*raw+x` transform | **implemented** | `generate_secret_in_seq_no` / `generate_secret_out_seq_no`, `:174-180`; the `x` values in `SecretChat.__init__` (`:62-71`) match the documented table |
| **parity check on receive** | **absent** | — |
| **`out_seq_no` continuity (`<= C` drop, `> C+1` gap)** | **absent** | `handle_decrypted_message` does `peer.in_seq_no += 1` under a literal `# TODO add checks` (`:326-327`) |
| **`in_seq_no` monotonicity and `<= D+1`** | **absent** | — |
| **gap queue + sending `Resend`** | **absent** | the package never constructs `decryptedMessageActionResend` |
| answering an inbound `Resend` | **present but incorrect** | `:309-317`. It re-sends via `send_secret_message(peer.id, message.message)`, i.e. a **new** message with a **new** `out_seq_no`, which cannot fill the peer's hole; `message.message` passes a `DecryptedMessage` object where `send_secret_message` expects `str`; and `peer.outgoing` is keyed one past the message's own `out_seq_no` (`:407` increments before `:409` stores) and is explicitly not persisted (`# TODO store these maybe too`, `:79`), so it is empty after any restart. No `MAX_RESEND_COUNT` bound |
| Resend handled immediately / exactly once | **absent** | no queue exists, so the exception rule has nothing to except |
| §3.8 delete-unacknowledged rewrite | **absent** | — |
| `invokeAfterMsgs` ordering of outgoing | **absent** | sends go out unchained |

This is the single largest gap: of the eight security checks §3.5–§3.6 make mandatory — each of which
the documentation ends with "the client is required to immediately abort the secret chat" — the
package performs **none**.

### §8.4 Rekeying (§4)

| Item | State | Evidence |
|---|---|---|
| 100-message / 1-week trigger | **implemented** | `:369-370` and `:388-389`, `ttr` counts down from 100, `time() - peer.updated > 7*24*60*60` |
| `RequestKey` / `AcceptKey` / `CommitKey` / `Noop` | **implemented** | `rekey` `:182`, `accept_rekey` `:201`, `commit_rekey` `:232`, `complete_rekey` `:263` |
| concurrent-`exchange_id` tie-break | **partial** | `:204-210` aborts when mine is larger and clears state when equal, but on the "mine is smaller" branch it falls through to `AcceptKey` **without reusing or discarding** the abandoned `(a, g_a)`, and it never sends `AbortKey` |
| `exchange_id` generation | **defective** | `random.randint(10000000, 99999999)` (`:189`) — the non-cryptographic `random` module, and only ~27 bits of an `int64` field, so the documented 2^-64 collision assumption of §4.7 does not hold |
| `AcceptKey` point-of-no-return | **not modelled** | nothing prevents an abort after `AcceptKey` |
| **`AbortKey` on receipt** | **absent** | it is sent (`:245`, `:270`) but falls into the `else: return decrypted_message` branch on receipt (`:319`), so a peer's abort never clears `peer.rekeying` and the chat can deadlock in a half-open exchange |
| retaining the previous key until gaps close | **absent** | `commit_rekey`/`complete_rekey` overwrite `peer.auth_key` outright (`:260`, `:277`); a message still in flight under the old key becomes undecryptable |
| `key_fingerprint` check on `AcceptKey` | **implemented** | `:244-250` |
| `key_fingerprint` check on `CommitKey` | **broken** | `:267` compares `self._temp_rekeyed_secret_chats[exchange_id]` — a **key (bytes)** — against `action.key_fingerprint`, an int. It can never be equal, and the guard above it (`:265`) returns early whenever the entry exists, so the comparison is dead code and the fingerprint is effectively unchecked |
| DH checks on rekey values | **implemented** | `check_g_a` is applied to the rekey `g_a`/`g_b` (`:218`, `:239`) — subject to the §8.1 gaps |
| original-key SHA-1 retained for visualization | **absent** | — |

### §8.5 Service actions (§5)

Generated schema coverage is **complete**: all thirteen `DecryptedMessageAction*` classes exist in
`secret_sechma/secretTL.py` (lines 250–606), matching `[DOC:schema/end-to-end]` exactly.

Runtime handling, `handle_decrypted_message` (`:286-334`):

| Action | Inbound | Outbound send method |
|---|---|---|
| `SetMessageTTL` (5.1) | sets `peer.ttl`, returns message | **absent** |
| `ReadMessages` (5.2) | pass-through to the app | **absent** |
| `DeleteMessages` (5.3) | pass-through | **absent** |
| `ScreenshotMessages` (5.4) | pass-through | **absent** |
| `FlushHistory` (5.5) | pass-through | **absent** |
| `Resend` (5.6) | present, incorrect (§8.3) | **absent** |
| `NotifyLayer` (5.7) | sets `peer.layer`, re-notifies, sets `mtproto=2` at >= 73 | `notify_layer()`, `:559` |
| `Typing` (5.8) | pass-through | **absent** |
| `RequestKey`/`AcceptKey`/`CommitKey` (5.9–5.11) | handled | handled |
| `AbortKey` (5.12) | **pass-through — not handled** (§8.4) | sent only on fingerprint failure |
| `Noop` (5.13) | ignored | sent in `complete_rekey` |

So of the six actions the consuming MCP server exposes as tools, **five have no send path at all**
(`SetMessageTTL`, `ReadMessages`, `DeleteMessages`, `ScreenshotMessages`, `Resend`) and only
`NotifyLayer` does.

### §8.6 Files (§6)

| Item | State | Evidence |
|---|---|---|
| fresh 32-byte `key` + 32-byte `iv` per file | **implemented** | `upload_secret_file`, `:458-459`, `os.urandom` |
| `md5(key+iv)` fingerprint, 4-byte XOR fold | **implemented** | `:460-463` (upload) and `:424-428` (download) |
| fingerprint verified before download | **implemented** | `:429-430`, raises `SecurityError` |
| `sendEncryptedFile` with `inputEncryptedFileUploaded` / `BigUploaded` | **implemented** | `:466-470` |
| download via `InputEncryptedFileLocation` | **implemented** | `:431-434` |
| forwarding an existing `inputEncryptedFile` | **absent** | |
| big-file (`size:long`, layer 143) awareness | **absent** | `DEFAULT_LAYER = 101` (`:31`) predates it |

Files are the healthiest area of the base package.

### §8.7 Layer and compatibility (§7)

| Item | State | Evidence |
|---|---|---|
| remote layer initialised to 46 | **violated** | `SecretChat.__init__` defaults `layer=DEFAULT_LAYER` = **101** (`:56`), so an un-notified peer is assumed to be at 101 |
| floor the outgoing layer at 73 | **absent** | `decryptedMessageLayer(layer=peer.layer, ...)` (`:394`) uses the raw remote value; TDLib clamps to `[73, 144]` (`[TD:td/telegram/SecretChatActor.h:649-658]`) |
| raise stored layer on any higher `layer_no` | **implemented** | `:329-330` |
| never lower the stored layer | **violated** | `peer.layer = decrypted_message.layer` and `peer.layer = action.layer` (`:298`) assign unconditionally, so a peer can walk its layer **down**; TDLib rejects that (§3.6) |
| `NotifyLayer` on chat creation | **implemented** | `accept_secret_chat` `:647`, `finish_secret_chat_creation` `:664` |
| `NotifyLayer` accepted in the layer-8 wrapper | **implemented** | sends `DecryptedMessageService8` (`:562`), accepts both on receipt |
| reject MTProto 1.0 from a peer at layer >= 73 | **absent** | see §8.2 |
| notify the user when the peer's layer exceeds ours | **absent** | |
| current layer | **stale** | `DEFAULT_LAYER = 101`; the oracle is at 144 |

### §8.8 Constitution-level findings in the inherited code

- **Principle IV (secret material never leaves its boundary).**
  `self._log.debug(f"outgoing peers {peer.outgoing}")` (`:315`) logs `DecryptedMessageLayer` objects,
  i.e. **plaintext message bodies**, at DEBUG. `SecretChat.__repr__` (`:101-115`) is safer — it omits
  `auth_key` — but it does emit `access_hash` and the full counter state, and it is interpolated into
  five log lines (`:184, 211, 237, 275, 284`).
- **Randomness.** `random.randint` appears in `rekey` (`:189`, `exchange_id`) and in both
  `random_bytes` length computations (`:396`, `:563`). Constitution requires `secrets`/`os.urandom` in
  every path producing key material, nonces or padding.
- **Principle III (test-first).** There is no test directory, no test file, and no CI configuration in
  the archive. Every inherited line is unverified.
- **File size.** `secret_methods.py` at 664 lines is already inside the constitution's "about 700 is
  closed to new code" band before any of the missing §3 logic is added. It has to be split by
  responsibility at the point of adoption, not later.

---

## §9. Open questions for the spec

The protocol does not decide these. The spec must.

1. **Storage backend shape.** The constitution forbids a silent fallback that writes keys to the CWD,
   so a backend must be chosen explicitly. What is the minimum interface — per-chat blob, or typed
   fields? What must be atomic? The base package persists via `SecretChat.__setattr__` calling
   `save()` on **every attribute write** (`:96-99`), which is both a write amplifier and a partial-write
   hazard; §4.8 requires two keys and §3.7 a message queue to survive a restart, neither of which the
   base package persists.
2. **Async API surface.** One manager object patched onto a `TelegramClient` (the base package's
   shape), or a standalone object owning its own update subscription? What is awaited, what is a
   callback, and does `send_secret_message` resolve when the server accepts or when the peer's
   `in_seq_no` acknowledges it?
3. **What happens on a failed decrypt.** The protocol says reject. The spec must say what "reject"
   means at the API boundary: raise, or deliver a typed failure event? Which failures abort the chat
   (§3.4 parity, §3.6 `in_seq_no`) versus drop the message (§3.5 replay) versus queue it (§3.5 gap)?
   And what the error carries, given Principle IV forbids formatting it from protocol objects.
4. **How the oracle tests are wired.** TDLib is available in-process only while the consuming server
   still runs it. What is the fixture format that outlives it — captured `(key, plaintext, padding,
   ciphertext, msg_key)` tuples, or a recorded session transcript? Which of §2.3, §2.4, §1.5, §6.2 get
   a vector each, and how is a live interop test gated in CI when it needs a real account?
5. **MTProto 1.0 boundary.** The constitution puts it out of scope, but §7.5 documents that early
   received messages may be v1. Does the package refuse them, or decode-and-discard them? A chat that
   cannot be established without v1 is presumably an error — the spec must name it.
6. **Layer ceiling.** Announce 144 to match the oracle, or a lower layer whose features are fully
   implemented? Announcing a layer implies being able to parse everything it introduced (§7.3).
7. **Telethon compatibility surface.** §8.0 lists five touch points, the most dangerous being
   `MTProtoState._calc_key`. Vendor the KDF (§2.4 is fully specified and ~10 lines) or keep depending
   on the private method with a canary test?
8. **Rekey policy.** The trigger is documented as client behaviour, not a requirement. Does this
   package rekey on the 100/7-day rule automatically, expose it as policy, or require the caller to
   drive it? And what happens to a `send` issued mid-exchange (§4.8)?
9. **Resend budget.** TDLib caps a resend span at 1000 (§3.7). Adopt that number, or derive one? What
   is the retention window for outgoing messages that makes a resend satisfiable at all — the
   protocol's answer to an unsatisfiable request is to abort the chat.
10. **TTL semantics.** §5 marks the countdown start as UNVERIFIED. The spec must either pin it with an
    interop observation or state explicitly that the package stores and transmits the TTL without
    enforcing it locally.
