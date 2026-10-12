# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Targets Holochain 0.7 (`hdk 0.7.0`, `hdi 0.8.0`). All commands run inside the dev shell: `nix develop` in the repo root.

```bash
npm install            # first time
npm run build:zomes    # wasm only; needs RUSTFLAGS='--cfg getrandom_backend="custom"' (set by the script)
npm run build:happ     # wasm + dnas/file_system/workdir/file_system.dna + workdir/soushi-cloud.happ
npm test               # build:happ, then cargo test (path unit tests + Sweettest)
npm run doc            # rustdoc, warnings are errors; the coordinator also enforces missing_docs
npm start              # kitsune2 bootstrap/relay on 127.0.0.1:4433 + 2 conductors (AGENTS=n to change)
```

**Run a single Sweettest scenario** (the DNA bundle must already be built):
```bash
cargo test -p file_system --test file_system deleted_file_leaves_its_directory_listing
```

## Architecture

This is a **Holochain application** ("Soushi Cloud") for distributed file storage. The core is a single DNA (`file_system`) with two Rust zomes compiled to WASM:

### Zome Structure

- **Integrity zome** (`dnas/file_system/zomes/integrity/file_system/`): Defines entry types, link types, and validation rules. Only the HDI (Holochain Deterministic Integrity) crate is available here, so no side effects.
- **Coordinator zome** (`dnas/file_system/zomes/coordinator/file_system/`): Handles all CRUD logic, path resolution, file chunking, and signals. Uses the full HDK.

### Entry & Link Types (defined in integrity zome)

- `FileMetadata`: stores file name, author pubkey, path, timestamps, size, file_type, and `chunks_hashes: Vec<EntryHash>`
- `FileChunk`: a raw byte array (`SerializedBytes`); files are split into 1 MB chunks

Link types:
- `PathFileSystem`: typed path anchors for the file system tree (uses Holochain's `Path` system)
- `PathToFileMetaData`: links a path entry to a `FileMetadata` action hash
- `FileMetaDataUpdate`: links original metadata hash → updated metadata hash (enables version chain traversal)

### Path System

File paths follow **two conventions** that must be kept in sync:
- **Filesystem paths**: standard `/` separator, e.g. `/subfolder/file.txt`
- **DHT paths**: `.` separator with `root` prefix, e.g. `root.subfolder`

Conversion is done by `fs_path_to_dht_path()` and `standardize_fs_path()` in `files.rs`. Forbidden characters in paths: `< > : " | ? * .`

### Versioning Model

Updates do **not** mutate entries. Instead:
1. New chunks are created; old chunks are deleted
2. A new `FileMetadata` entry is created via HDK `update_entry`
3. A `FileMetaDataUpdate` link connects original hash → new hash
4. `get_file_metadata()` always resolves to the latest version by traversing these links

### Signals (`signals.rs`)

The `post_commit` hook emits signals after every committed action:
- `FileMetadataCreated` / `FileMetadataUpdated` / `FileMetadataDeleted`
- Only emits for `FileMetadata` entries, not chunks

### Tests

Sweettest (Rust, in-process conductors) in `dnas/file_system/zomes/coordinator/file_system/tests/file_system.rs`. Each scenario runs Alice and Bob on two conductors sharing one DNA, loaded from `dnas/file_system/workdir/file_system.dna`, and uses `await_consistency` before cross-agent reads. Path conversion unit tests live in `files.rs`. Tryorama is not used: it has no Holochain 0.7 release.

### Build Artifacts

The Cargo workspace compiles two WASM targets:
- `target/wasm32-unknown-unknown/release/file_system_integrity.wasm`
- `target/wasm32-unknown-unknown/release/file_system.wasm`

These are bundled into `workdir/soushi-cloud.happ` by the `hc app pack` command.
