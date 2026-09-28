# 0003. No MTProto 1.0 code path, and no runtime refusal of an old peer

- Status: accepted
- Date: 2026-09-28 (decided 2026-09-19; the refusal left unwired on 2026-09-26)

## Context

MTProto 2.0 starts at layer 73 (§7). The archived base package defaulted to 1.0 for the first
messages of every chat and permanently downgraded on any decryption error, a downgrade an
attacker could cause by corrupting one byte (§8.2). §7.5 notes early messages from some peers
may be 1.0. The feature spec also asked for an explicit refusal of a peer below layer 73.

## Decision

`crypto.py` implements MTProto 2.0 only: no version parameter, no fallback, nothing that
produces or accepts a 1.0 frame. The explicit refusal is not wired (owner decision of
2026-09-26: peers below layer 73 are out of scope), and a frame this package cannot decrypt
already surfaces as `DecryptFailed` (ADR 0002). `LayerUnsupported` stays exported so the public surface does
not change if the refusal is ever wired.

## Consequences

- There is no downgrade to attack.
- A peer stuck below layer 73 sees its messages refused one by one rather than a single
  named refusal; the README states this.
- `LayerUnsupported` is documented as not raised today.
