# Abbey security patch

This directory is the unmodified `openmls_rust_crypto` 0.5.1 crate published
on crates.io, except for its normalized and original Cargo manifests.
The original crates.io archive SHA-256 is
`fafcc8a3552b10fbb3ab757cccaf1a34081e826ca819f49aa7e6645b1d95c00f`,
matching the registry checksum recorded for version 0.5.1.

The manifests select `hpke-rs`, `hpke-rs-crypto`, and
`hpke-rs-rust-crypto` 0.8 instead of 0.6. The locked 0.8 line uses the fixed
`libcrux-sha3` 0.0.11 and `libcrux-kem` 0.0.10 dependencies and removes the
vulnerable SHA3 and KEM versions from Abbey's resolved DAVE dependency graph
while preserving the
0.5.1 API required by `davey` 0.1.4.

Source provenance is retained in `.cargo_vcs_info.json`; the original crate
license and README are included unchanged. This patch must be removed when
`davey` publishes a release using the fixed OpenMLS/HPKE line.

On 2026-10-08 the prior 0.7 line was replaced because its exact
`libcrux-kem = 0.0.9` pin carried RUSTSEC-2026-0330 and RUSTSEC-2026-0331.
The 0.8 update changes the three existing HPKE dependencies and their
transitive cryptographic dependencies; it adds no direct application dependency.
The five previously reviewed RustSec vulnerabilities remain accepted debt.
