# Security policy

## Supported versions

Security fixes are provided for the newest published Quick Share 2.x release. The `2.0.0-alpha.0` development line is not yet approved for production release.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub Security Advisories for `Newbluecake/quick-share` or contact the repository owner privately. Include:

- affected version and platform;
- reproduction steps or proof of concept;
- expected security boundary;
- whether private keys, authorization tokens, release signatures, or user files may be exposed.

Avoid including real private keys, access tokens, upload passwords, or sensitive transferred content.

## Release trust

Release binaries are accompanied by:

- `SHA256SUMS`;
- `SHA256SUMS.sig`, an Ed25519 signature over the exact checksum manifest;
- SPDX JSON SBOMs;
- GitHub build provenance attestations.

The updater pins `security/release-signing-key.pem`. Release signing private material is stored only in the protected `RELEASE_SIGNING_KEY_PEM` GitHub Actions secret.

Key rotation uses an overlap release: first publish a release signed by the old key that contains both old and new public keys, then change the Actions secret, and only later remove the old public key. Never replace the key and signing secret in the same first release.

## Known release gate

The direct-transfer implementation currently uses `snow 0.10.0`, which states that it has not received a formal third-party security audit. A dedicated expert review or an audited replacement remains mandatory before approving the final v2.0.0 release.
