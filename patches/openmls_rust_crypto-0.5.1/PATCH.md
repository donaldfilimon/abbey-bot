# Abbey security patch

This directory is the unmodified `openmls_rust_crypto` 0.5.1 crate published
on crates.io, except for its normalized and original Cargo manifests.
The original crates.io archive SHA-256 is
`fafcc8a3552b10fbb3ab757cccaf1a34081e826ca819f49aa7e6645b1d95c00f`,
matching the registry checksum recorded for version 0.5.1.

The manifests select `hpke-rs`, `hpke-rs-crypto`, and
`hpke-rs-rust-crypto` 0.7 instead of 0.6. The 0.7 line uses the fixed
`libcrux-sha3` 0.0.10 dependency and removes the vulnerable 0.6/libcrux
versions from Abbey's active DAVE dependency graph while preserving the
0.5.1 API required by `davey` 0.1.4.

Source provenance is retained in `.cargo_vcs_info.json`; the original crate
license and README are included unchanged. This patch must be removed when
`davey` publishes a release using the fixed OpenMLS/HPKE line.
