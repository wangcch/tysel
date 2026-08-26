# Verify and sign an application release

This guide builds one Tysel application artifact set, validates its evidence
through the signing path, and records the boundaries between deterministic
application evidence and the Tysel toolchain's own two-build release process.

## Build on the deployment target

Tysel does not cross-compile application executables. Run the admission checks
on the same operating system and architecture used in production:

```sh
tysel doctor --install
tysel task verify
tysel inspect
tysel build --release --output dist/orders
```

The output is one immutable set:

| File | Purpose |
| --- | --- |
| `orders` | Native executable with its embedded application package. |
| `orders.sha256` | SHA-256 of the complete executable. |
| `orders.compat.json` | Runtime and package compatibility decision. |
| `orders.sbom.cdx.json` | CycloneDX 1.5 software bill of materials. |
| `orders.licenses.json` | Runtime component license inventory. |
| `orders.evidence.json` | Deterministic index binding artifact identity, target, application, compatibility, SBOM, licenses, and runtime inventory. |

If sidecar generation fails, no valid evidence index should be admitted. Store
all six files together under the executable digest. Do not edit, rename
independently, or regenerate only one sidecar after signing.

## Validate and sign the set

`tysel release sign` takes the application executable, first verifies the
executable and all release sidecars against one another, and then signs the
evidence index with Ed25519:

```sh
tysel release key-info --key /secure/offline-release.key
tysel release sign dist/orders --key /secure/offline-release.key
```

The second command creates `orders.evidence.sig.json`. Keep the private key
outside source control, build directories, command logs, and runtime hosts.
`key-info` prints the public key and derived key ID for constructing your
time-bounded trust policy; it does not make the private key safe to distribute.

At admission, verify with the trusted public-key policy:

```sh
tysel release verify dist/orders --trust deploy/trust.json
```

Verification checks the complete sidecar set again, the evidence digest,
signature algorithm and key ID, key status and validity, policy lifetime, and
signature time. A revoked key, expired policy, future signature, changed byte,
missing sidecar, or inconsistent SBOM/license inventory fails closed.

`release verify` authenticates the target recorded by the build, but it does
not accept an expected deployment target. After signature verification, make
that comparison an explicit admission gate:

```sh
deployment_target=linux-x64
test "$(jq -r '.artifact.target' dist/orders.evidence.json)" = "$deployment_target"
```

Use the canonical target selected by the deployment, not a value supplied by
the artifact producer. Reject an empty, unknown, or mismatched value before
copying the executable or building its container image.

Artifact signing in an OCI registry is a separate deployment control. Apply
your organization's image or registry signing after `tysel image`, and bind
the container digest back to the verified application digest in the change
record.

## Record reproducible inputs

The application evidence is deterministic for a produced executable, but
`tysel build --release` does not claim that two arbitrary hosts will emit
byte-identical application binaries. Record at least:

- application source commit and manifest revision;
- lockfile digest and installed Tysel version;
- canonical build target and build command;
- executable and evidence-index digests;
- base-image digest when deploying a container;
- signing key ID, trust-policy digest, and verification time.

Never record private keys, secret values, Postgres URLs, OTLP headers, or
provider credentials.

## Application reproducibility boundary

Application evidence proves that the executable and sidecars belong together;
it does not assert that arbitrary build hosts emit byte-identical binaries.

## Recover from verification failures

| Failure | Response |
| --- | --- |
| Executable or sidecar digest mismatch | Quarantine the entire set and rebuild; never patch evidence in place. |
| Compatibility report mismatch | Rebuild with one admitted Tysel release and re-run compatibility review. |
| Missing SBOM or license inventory | Reject admission and restore the complete set from the immutable store. |
| Signed target differs from deployment target | Reject admission and build on the intended operating system and architecture. |
| Trust policy expired | Authenticate and publish a new forward-valid policy; do not change clocks or bypass verification. |
| Signing key revoked | Stop deployments using it, rotate offline, rebuild or re-sign retained verified artifacts under policy. |

See [Release evidence commands](../reference/cli/evidence.md),
[Build command](../reference/cli/delivery.md#tysel-build), and
[Production build admission](../operations/production.md#build-admission).
