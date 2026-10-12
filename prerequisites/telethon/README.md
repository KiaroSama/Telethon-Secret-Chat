# Project-owned Telethon prerequisite

This directory tracks **only reviewed patches, extension source and provenance**.
It does not vendor the complete upstream repository. The build tool downloads the
exact official Telethon 1.45.0 wheel, verifies its SHA-256 and each edited module,
applies every hunk exactly once, adds the extension, and rebuilds valid wheel
metadata/RECORD as `telethon==1.45.0+kiaro.ingress1`.

Upstream: https://codeberg.org/Lonami/Telethon
Release tag: `v1.45.0`, commit `bad095bd5b3dc9f852cfbeeba300a421169c9b1e`.
Archive and module identities are in `upstream.json` and `patches.json`.
The Codeberg source and official release wheel module hashes were compared.
The wheel is MIT; its original license stays in the rebuilt distribution.

## Build and installation

```bash
python tools/build_telethon.py --output dist/prerequisites
```

For an offline build, pass `--archive <cached-official-wheel>`; checksum verification
is still mandatory. Install the resulting wheel normally in the caller's selected
virtual environment. This replaces the official Telethon distribution **in that
environment only**, retaining the `telethon` import name. It does not edit an
already-installed package or a running client. Never install official and renamed
copies with the same import namespace side by side.

Normal dependency syncing currently selects the official release from `uv.lock`.
Development verification with the patched dependency uses `uv run --no-sync` after
explicit installation. CI builds and installs the prerequisite after syncing.
There is no global pip post-install hook or silent replacement on arbitrary future
Telethon upgrades: an unknown archive, changed module or ambiguous patch refuses.

## Current availability

**Release validation is pending.** The public extension now validates attachment
to the exact prepared coordinator before connection. Unknown versions, incompatible
contracts, wrong identities and missing checkpoints refuse. Final lifecycle, fault,
mixed-stream, backpressure and installed-wheel acceptance must still pass. Source tests can exercise the in-development contract
with non-sensitive fixtures; that is not account activation or live interoperability.

The caller owns the original client and authorization. Initial setup is before
connection, with an explicit verified baseline, an authenticated encrypted store,
a caller-owned encryption key and exclusive store ownership. No session migration,
new login/device, consumer installation or deployment is performed here.

The independent `tests/wheel_ingress_smoke.py` checks installed module hashes,
distribution version, public extension shape and refusal of incomplete contracts
outside the source checkout. Ordinary and transient APIs must remain compatible.
