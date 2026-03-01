# Implementation Specification — File System Zome Refactor

## Goal

Evolve this project from a proof-of-concept using outdated HDK into a production-ready, embeddable Holochain zome that serves as the file system layer for the Technology Trinity (Nondominium, hAppenings, IDI).

## Target State

- Compiles against Holochain 0.4.x (current stable HDK/HDI)
- Delegates chunk operations to `holochain-open-dev/file-storage`
- All known bugs fixed
- All existing tests passing
- Published as a reusable crate importable by other DNAs

---

## Phase 1 — HDK Migration

**Objective:** Get the project compiling against modern Holochain.

### Steps

**1.1 — Update Cargo.toml versions**

Update `dnas/file_system/zomes/integrity/file_system/Cargo.toml`:
```toml
[dependencies]
hdi = { workspace = true }
```

Update `dnas/file_system/zomes/coordinator/file_system/Cargo.toml`:
```toml
[dependencies]
hdk = { workspace = true }
file_system_integrity = { path = "../../integrity/file_system" }
```

Update root `Cargo.toml` workspace dependencies to current Holochain versions. Check [Holochain releases](https://github.com/holochain/holochain/releases) for latest stable `0.4.x` versions.

**1.2 — Resolve API breaking changes**

Key changes between HDK v0.1.x and HDK v0.3+:
- `create_entry` signature changed — verify `EntryTypes` usage compiles
- Path API: verify `typed()`, `ensure()`, `children_paths()` signatures
- `get_details` return types — verify `Details::Record` destructuring
- `emit_signal` — verify signal serialization format
- `wasm_error!` macro — verify usage is current

**1.3 — Fix tests**

Update `tests/package.json` to use matching `@holochain/tryorama` and `@holochain/client` versions compatible with the new Holochain conductor version.

Run the full test suite and fix any failures:
```bash
npm test
```

**Acceptance criteria:**
- `npm run build:zomes` succeeds
- `npm test` passes all existing tests

---

## Phase 2 — Community Zome Integration

**Objective:** Delegate chunk storage to `holochain-open-dev/file-storage`, removing the duplicate chunk implementation.

### Steps

**2.1 — Add community zome dependency**

In the coordinator `Cargo.toml`:
```toml
[dependencies]
hc_zome_file_storage_integrity = { git = "https://github.com/holochain-open-dev/file-storage", tag = "<latest>" }
```

**2.2 — Update `dna.yaml` to include community zome**

In `dnas/file_system/workdir/dna.yaml`, add the community integrity and coordinator zomes alongside the file system zomes. This gives the coordinator access to call the community zome's functions.

**2.3 — Refactor `chunk_file()` to use community zome**

Replace the local `create_file_chunk()` implementation with a call to the community zome's `create_file_chunk`:

```rust
// Before (local implementation)
fn create_file_chunk(file_chunk: FileChunk) -> ExternResult<Record> { ... }

// After (delegating to community zome)
fn create_file_chunk_via_community(chunk_bytes: Vec<u8>) -> ExternResult<EntryHash> {
    let result = call(
        CallTargetCell::Local,   // or cross-DNA bridge
        "file_storage",
        "create_file_chunk".into(),
        None,
        FileChunk(SerializedBytes::from(UnsafeBytes::from(chunk_bytes))),
    )?;
    // decode and return EntryHash
}
```

**2.4 — Refactor `get_file_chunk()` to use community zome**

Similarly, delegate `get_file_chunk` calls to the community zome's coordinator.

**2.5 — Remove local `FileChunk` entry type from integrity zome**

Once chunk operations are fully delegated, remove `FileChunk` from this project's `EntryTypes` enum and the `FileChunk` struct from the integrity zome. The community integrity zome owns this type.

**2.6 — Update `chunks_hashes` interpretation**

`FileMetadata.chunks_hashes` stores `EntryHash` values pointing to `FileChunk` entries in the community zome. No structural change to `FileMetadata` is needed — the hashes remain valid references regardless of which zome stores the entries.

**Acceptance criteria:**
- All chunk operations succeed via community zome calls
- Local `FileChunk` entry type removed from integrity zome
- `npm test` passes all existing tests

---

## Phase 3 — Bug Fixes

**Objective:** Fix all known issues identified in the current implementation.

### Bug 3.1 — Path links not cleaned up on file deletion

**Location:** `coordinator/file_system/src/lib.rs` → `delete_file()`

**Problem:** When a file is deleted, the `PathToFileMetaData` link from the directory path to the file's action hash is not removed. Deleted files continue to appear in directory listings.

**Fix:** In `delete_file()`, after deleting the metadata entries, retrieve and delete the `PathToFileMetaData` link.

Approach:
1. Get the `FileMetadata` from the original hash to find the file's path
2. Convert the path to DHT path
3. Get all `PathToFileMetaData` links from that path entry
4. Find the link whose target matches `original_file_metadata_hash`
5. Delete that link

**Acceptance criteria:**
- After `delete_file()`, `get_files_metadata_by_path_recursively()` no longer returns the deleted file

### Bug 3.2 — Debug println in path conversion

**Location:** `coordinator/file_system/src/files.rs` → `standardize_fs_path()`

**Problem:** A `println!("path: {:?}", path)` statement produces noisy output in WASM.

**Fix:** Remove the `println!` statement.

```rust
// Remove this line:
println!("path: {:?}", path);
```

**Acceptance criteria:**
- No path-related stdout output during normal operation

---

## Phase 4 — New Capabilities

**Objective:** Add missing functionality needed by the Technology Trinity.

### Feature 4.1 — File Move / Rename

**Motivation:** Nondominium governance flows may require reorganizing documents. IDI curriculum restructuring requires moving learning content between paths.

**New function:** `move_file(MoveFileInput) → Record`

```rust
pub struct MoveFileInput {
    pub original_file_metadata_hash: ActionHash,
    pub new_path: String,
    pub new_name: Option<String>,  // None = keep existing name
}
```

**Implementation:**
1. Get current `FileMetadata` (latest version)
2. Create updated `FileMetadata` with new `path` (and `name` if provided)
3. Create a `FileMetaDataUpdate` link
4. Create new `PathToFileMetaData` link at the new path
5. Delete old `PathToFileMetaData` link at the old path

**Note:** This will require relaxing the immutability of `path` and `name` in the current validation logic. The integrity zome validation should be updated to allow these fields to change on update.

### Feature 4.2 — Directory Listing (Non-Recursive)

**Motivation:** UI clients need to browse one level at a time (like a file explorer), not always pull all files recursively.

**New function:** `get_files_metadata_by_path(path_string: String) → Vec<Record>`

Lists only files directly at the given path (no subdirectories), plus the names of immediate subdirectories.

**New type:**
```rust
pub struct DirectoryListing {
    pub files: Vec<Record>,
    pub subdirectories: Vec<String>,
}
```

---

## Phase 5 — Publication

**Objective:** Make the zome importable as a reusable crate.

### Steps

**5.1 — Publish integrity crate**

Publish `file_system_integrity` to crates.io:
- Crate name: `hc_zome_file_system_integrity`
- Ensure all public types are properly documented
- Include `CHANGELOG.md`

**5.2 — Publish coordinator crate**

Publish the coordinator as `hc_zome_file_system`.

**5.3 — Update CLAUDE.md**

Update the development commands section to reflect the new dependency structure and any new npm scripts.

---

## Implementation Order

| Phase | Effort | Dependency | Priority |
|-------|--------|------------|----------|
| Phase 1 — HDK Migration | Medium | None | Critical (blocks everything) |
| Phase 3 — Bug Fixes | Small | Phase 1 | High (correctness) |
| Phase 2 — Community Integration | Medium | Phase 1 | High (architecture target) |
| Phase 4.2 — Directory Listing | Small | Phase 1 | Medium (UX improvement) |
| Phase 4.1 — Move/Rename | Medium | Phase 2, Phase 3 | Medium (feature completeness) |
| Phase 5 — Publication | Small | All above | Low (after stabilization) |

Start with Phase 1 — nothing else is possible until the HDK compiles.

---

## Testing Strategy

All phases must maintain a passing test suite. The test matrix:

| Test | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
|------|---------|---------|---------|---------|
| create files and get by path | pass | pass | pass | pass |
| create large file and delete | pass | pass | must include path link cleanup | pass |
| create, update, read, cascade delete | pass | pass | pass | pass |
| empty name and path standardization | pass | pass | pass | pass |
| move file (new) | — | — | — | add |
| directory listing non-recursive (new) | — | — | — | add |

Run tests after each phase:
```bash
npm test
```
