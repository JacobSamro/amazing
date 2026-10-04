# Linux native Cargo registry

The public launcher previously moved workspace sources and targets to `/tmp` but left Cargo registry sources on snapshot-backed HOME. The preserved failing nextest log shows OpenH264 source/header reads failing with `Transport endpoint is not connected`; Opus CMake also read its source tree from HOME. The `-undefined dynamic_lookup` argument originates in OpenH264’s build script and was not the demonstrated failure cause.

The launcher now initializes a native per-lane `CARGO_HOME` after loading the actual Linux environment. Cargo downloads and unpacks locked packages there. It does not copy registry trees from HOME or change dependencies, compiler flags, native library environment, Rustup roots, or project Cargo configuration. Directory admission rejects symlink/non-native storage. Native Cargo caches are ephemeral across sandbox stops.

Qualification on source `01b17fe2`, recorded base `14853c4e`: nine public tooling fixtures passed; all 318 registry package manifests resolved under native Cargo storage; actual locked OpenH264 and Opus builds passed. Each of the three phases and the owner returned zero. Raw results remain on Boat; compact hashes and source identity are in [qualification.summary.json](evidence/linux-native-registry/qualification.summary.json). This does not qualify the held product viewport or painter tickets.

Run: `scripts/linux-run.sh js-lang --ref <exact-source> -- 'python3 scripts/linux-native-cargo-fixture.py'`.
