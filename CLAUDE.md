# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

All commands must be run from inside the Nix shell (`nix-shell` in the root directory).

```bash
# Install dependencies (first time)
npm install

# Build Rust WASM zomes only
npm run build:zomes
# Equivalent: RUSTFLAGS='' CARGO_TARGET_DIR=target cargo build --release --target wasm32-unknown-unknown

# Build the full hApp (zomes + pack)
npm run build:happ

# Run all tests (builds zomes, packs hApp, runs vitest)
npm test

# Run tests within the tests/ workspace directly (requires hApp to already be built)
npm t -w tests

# Start a 2-agent network with UI
npm start

# Package the web hApp for distribution
npm run package
```

**Run a single test** by passing a test name filter to vitest:
```bash
cd tests && npx vitest run --reporter=verbose -t "create file with empty name"
```

## Architecture

This is a **Holochain application** ("Soushi Cloud") for distributed file storage. The core is a single DNA (`file_system`) with two Rust zomes compiled to WASM:

### Zome Structure

- **Integrity zome** (`dnas/file_system/zomes/integrity/file_system/`): Defines entry types, link types, and validation rules. Only the HDI (Holochain Deterministic Integrity) crate is available here — no side effects.
- **Coordinator zome** (`dnas/file_system/zomes/coordinator/file_system/`): Handles all CRUD logic, path resolution, file chunking, and signals. Uses the full HDK.

### Entry & Link Types (defined in integrity zome)

- `FileMetadata`: stores file name, author pubkey, path, timestamps, size, file_type, and `chunks_hashes: Vec<EntryHash>`
- `FileChunk`: a raw byte array (`SerializedBytes`) — files are split into 1 MB chunks

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

### Tests (`tests/`)

TypeScript integration tests using `@holochain/tryorama` and `vitest`. Tests run two agents (Alice and Bob) with a shared DHT. The compiled hApp must exist at `../workdir/soushi-cloud.happ` before tests run (handled automatically by `npm test`).

Helper functions in `tests/src/file_system/common.ts` wrap all zome calls.

### Build Artifacts

The Cargo workspace compiles two WASM targets:
- `target/wasm32-unknown-unknown/release/file_system_integrity.wasm`
- `target/wasm32-unknown-unknown/release/file_system.wasm`

These are bundled into `workdir/soushi-cloud.happ` by the `hc app pack` command.
