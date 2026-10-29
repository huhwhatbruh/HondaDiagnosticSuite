**What changed and why**


**How you checked it**
- [ ] `cargo test` passes in the crate(s) you touched
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] It runs against the simulated car (`hondascan -a sim scan`)

**Safety**
- [ ] The read-only release still refuses everything outside its allowlist
- [ ] No VIN, capture or personal path is in the diff
