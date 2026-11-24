# Publisher signatures

Kiaro Secret Chat signs native packages as `CN=KiaroSama, OU=Kiaro Secret Chat`. The [publisher record](signing/publisher.json) pins the self-signed RSA-3072 certificate and its public key. Get that record from a trusted copy of the repository. Use it to check any `publisher-cert.pem` or `publisher-key.pem` supplied with a download.

## What CI signs

The Rust workflow signs x64 and arm64 packages on Linux and Windows. On Windows, SignTool adds a SHA-256 Authenticode signature and an RFC 3161 timestamp from DigiCert to each tool: `generate-schema.exe`, `matrix-evidence.exe`, `migration-audit.exe`, and `prepare-sdk.exe`. CI verifies each executable before it rebuilds the ZIP and its internal `SHA256SUMS`.

Each final `.zip` or `.tar.gz` gets a `.sha256` checksum and a detached RSA-3072 PKCS#1 v1.5 SHA-256 `.sig` signature. The workflow checks that the archive signature rejects a modified copy. For new version tags, the publish job checks that the tag matches the Cargo version, signs the release `SHA256SUMS`, and creates GitHub build provenance attestations for the archives. Create the matching GitHub release before the publish job uploads the assets.

The [`v0.3.0` release](https://github.com/KiaroSama/Telethon-Secret-Chat/releases/tag/v0.3.0) keeps its original GPG-signed tag and `.asc` signatures. Use the published GPG key for those files; this publisher certificate applies to the new signing workflow.

## Verify an archive

Use the pinned public key, not an unchecked key from the same download. From the repository root, extract it with `jq`, then run the remaining commands where the downloaded files are stored:

```bash
jq -er .public_key_pem docs/signing/publisher.json > publisher-key.pem
openssl dgst -sha256 -verify publisher-key.pem \
  -signature native-x86_64-pc-windows-msvc.zip.sig native-x86_64-pc-windows-msvc.zip
```

OpenSSL must report `Verified OK`. If you downloaded a single archive, check its `.sha256` sidecar with `sha256sum -c native-x86_64-pc-windows-msvc.zip.sha256`. If you downloaded the complete release, verify the signed manifest first, then check all its files:

```bash
openssl dgst -sha256 -verify publisher-key.pem -signature SHA256SUMS.sig SHA256SUMS
sha256sum -c SHA256SUMS
```

After extracting a package, run `sha256sum -c SHA256SUMS` from inside the extracted directory to verify its contents.

## Verify the Windows executables

In PowerShell, replace the paths below with your trusted publisher record and extracted package directory. Check every tool:

```powershell
$publisher = Get-Content -LiteralPath './docs/signing/publisher.json' -Raw | ConvertFrom-Json
foreach ($name in 'generate-schema', 'matrix-evidence', 'migration-audit', 'prepare-sdk') {
    $sig = Get-AuthenticodeSignature -LiteralPath "./native-tools/$name.exe"
    if ($sig.SignerCertificate.Thumbprint -cne $publisher.certificate_thumbprint -or
        $null -eq $sig.TimeStamperCertificate -or $sig.Status -eq 'HashMismatch') {
        throw "Unexpected Authenticode signature: $name.exe"
    }
    [pscustomobject]@{ File = "$name.exe"; Status = $sig.Status; Timestamp = $sig.TimeStamperCertificate.Subject }
}
```

Windows may report the publisher as untrusted because the certificate is self-signed. That does not establish that a file is intact. Verify the archive signature and internal checksums before running the tools. Do not import the certificate into a trusted store just to suppress a warning.

## Verify GitHub build provenance

```bash
gh attestation verify native-x86_64-pc-windows-msvc.zip --repo KiaroSama/Telethon-Secret-Chat
```

This checks the archive against the GitHub workflow's recorded build provenance. The OpenSSL check uses the separate publisher key.
