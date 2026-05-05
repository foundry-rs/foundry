# Security Policy

## Reporting a Vulnerability

Contact [security@tempo.xyz](mailto:security@tempo.xyz).

## Verifying Releases

Every official Foundry release ships with multiple, independent integrity
artifacts. All signing is keyless via [Sigstore](https://www.sigstore.dev/) —
no Foundry-managed key material is involved, and every signature is recorded
in the public [Rekor](https://docs.sigstore.dev/logging/overview/) transparency
log. The signing identity is the GitHub Actions OIDC token of this repository's
`release.yml` / `docker-publish.yml` workflows.

### Per-release artifacts

For each `foundry_<version>_<platform>_<arch>.{tar.gz,zip}` archive on the
[releases page](https://github.com/foundry-rs/foundry/releases), the same
release also publishes:

| Suffix | Purpose |
| --- | --- |
| `.sha256` | SHA-256 checksum of the archive (`sha256sum` format) |
| `.sigstore.json` | Cosign keyless signature bundle (cert + signature + Rekor proof) over the archive |
| `.spdx.json` | SPDX 2.3 SBOM of the source workspace used for the build |
| `.attestation.txt` | URL of the GitHub artifact-attestation summary |

In addition, GitHub stores SLSA build-provenance and SBOM attestations against
the archive's digest; these are queryable via `gh attestation` without
downloading anything else.

### Verifying an archive

Pick whichever toolchain you have available — they verify the same signatures.

#### Option 1: GitHub CLI (simplest)

```bash
gh attestation verify foundry_v1.4.0_linux_amd64.tar.gz \
  --repo foundry-rs/foundry
```

This computes the file's digest, fetches the matching attestation from GitHub,
and verifies the Sigstore signature plus the SLSA provenance predicate. Add
`--signer-workflow foundry-rs/foundry/.github/workflows/release.yml` to also
require the workflow identity.

To verify the SBOM attestation specifically:

```bash
gh attestation verify foundry_v1.4.0_linux_amd64.tar.gz \
  --repo foundry-rs/foundry \
  --predicate-type 'https://spdx.dev/Document/v2.3'
```

#### Option 2: Cosign (offline-friendly)

Download the archive and its `.sigstore.json` bundle from the release page,
then:

```bash
cosign verify-blob \
  --bundle foundry_v1.4.0_linux_amd64.sigstore.json \
  --certificate-identity-regexp '^https://github.com/foundry-rs/foundry/\.github/workflows/release\.yml@.*' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  foundry_v1.4.0_linux_amd64.tar.gz
```

For nightly builds the certificate identity points at `refs/heads/master`
instead of a tag; the regex above matches both.

#### Option 3: Plain checksum (integrity only)

```bash
sha256sum -c foundry_v1.4.0_linux_amd64.sha256       # GNU coreutils
shasum -a 256 -c foundry_v1.4.0_linux_amd64.sha256   # macOS
```

This proves the bytes match what was uploaded, but says nothing about who
uploaded them. Combine with one of the verifications above for end-to-end
trust.

### Verifying the Docker image

Container signatures and attestations are pushed as OCI referrers to GHCR, so
no separate files need to be downloaded.

```bash
# Cosign keyless signature on the image
cosign verify ghcr.io/foundry-rs/foundry:v1.4.0 \
  --certificate-identity-regexp '^https://github.com/foundry-rs/foundry/\.github/workflows/(release|docker-publish)\.yml@.*' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com'

# SLSA build-provenance attestation
gh attestation verify oci://ghcr.io/foundry-rs/foundry:v1.4.0 \
  --repo foundry-rs/foundry

# Inspect the buildx-attached SBOM and provenance
docker buildx imagetools inspect ghcr.io/foundry-rs/foundry:v1.4.0 \
  --format '{{ json .SBOM }}'
docker buildx imagetools inspect ghcr.io/foundry-rs/foundry:v1.4.0 \
  --format '{{ json .Provenance }}'
```

To pin to an immutable digest (recommended for reproducible deployments):

```bash
docker pull ghcr.io/foundry-rs/foundry:v1.4.0
DIGEST=$(docker buildx imagetools inspect ghcr.io/foundry-rs/foundry:v1.4.0 --format '{{ .Manifest.Digest }}')
cosign verify "ghcr.io/foundry-rs/foundry@${DIGEST}" \
  --certificate-identity-regexp '^https://github.com/foundry-rs/foundry/\.github/workflows/(release|docker-publish)\.yml@.*' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com'
```
