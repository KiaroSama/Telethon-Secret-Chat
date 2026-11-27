# Interoperability testing tiers

`Grammers Secret Chat` separates cryptographic/protocol verification into three distinct tiers so offline checks are never confused with live-server or official-client interoperability.

## Tier 1: Offline vectors (default CI)

Runs automatically on every push and pull request with zero credentials:

- MTProto 2.0 KDF, AES-256-IGE, DH validation, framing, sequence counters, PFS rekey, and replay vectors (`tests/protocol_vectors.rs`, `tests/protocol_audit.rs`, `tests/protocol_followup.rs`, `tests/parity_rekey_restart_audit.rs`).
- Synthetic two-party Alice/Bob establishment and message exchange (`tests/transport_establishment.rs`).
- Offline replay fixtures use synthetic or burned throwaway test keys only.

## Tier 2: Same-library live pair (opt-in manual)

Exchanges real encrypted messages through Telegram's servers between two controlled test accounts owned by the operator, both running `kiaro-secret-chat`:

- Proves real DC routing, Layer 234 TL encoding, server DH parameters, and live update delivery.
- Does **not** by itself prove interoperability with independent official Telegram clients.

## Tier 3: Independent official-client interoperability (opt-in manual)

Exchanges encrypted messages, media, timers, read receipts, and PFS rekey frames between `kiaro-secret-chat` and an independent official client (Telegram Android, iOS, macOS, or Desktop) on a controlled throwaway chat:

- Proves real cross-implementation agreement on key visualization (`12x12` picture + 64-char hex), MTProto 2.0 framing, layer negotiation, and encrypted file transfer.

## Running the opt-in workflow safely

Trigger `.github/workflows/live-interop.yml` manually via **Actions -> Live Interoperability (Opt-In) -> Run workflow** and pick `offline-vectors`, `same-library-pair`, or `official-client-interop`.

Credential rules:
- Store `TELEGRAM_INTEROP_API_ID`, `TELEGRAM_INTEROP_API_HASH`, `TELEGRAM_INTEROP_SESSION_A`, and (for pair mode) `TELEGRAM_INTEROP_SESSION_B` only in GitHub Actions Secrets or local gitignored storage.
- Never commit `.session`, `.env.live`, or auth keys to Git; `.github/scripts/validate-source.sh` and Gitleaks block them.
- The workflow fails closed if live tiers are selected without configured secrets rather than reporting a false pass.
