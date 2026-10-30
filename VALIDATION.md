# Publication validation

Public-source clone (without proprietary fixtures):

- `cargo clippy --locked --features offline-cli --all-targets -- -D warnings`: passed.
- `cargo build --locked --release --features offline-cli`: passed.
- `cargo test --locked --features offline-cli`: compiled; 31 unit tests passed, 17 failed with Windows `Os { code: 5, kind: PermissionDenied }` when creating files/directories under the host temporary directory. Integration tests were not reached. This was not a clean all-pass test run; no permissions were modified to bypass it.

In the original development workspace, the full fixture-backed regression suite passed before publication, including source/THK roundtrips, independent table learning, runtime boundary preservation, sandboxed scripting, strategy bundles and automatic missing-directory creation. That earlier result is not a substitute for a clean clone test pass on this host.

Game binaries, extracted THKs, generated THKs, captured player statistics and local package archives are deliberately not published. Private fixture tests are opt-in via `game-fixtures`. The historical `tests/script_learning.rs` depended on captured player data and is excluded from the public source snapshot.
