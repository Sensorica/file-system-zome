# Architecture — File System Zome

## Design Philosophy

This zome implements a **separation of concerns** between two distinct responsibilities:

1. **Chunk storage** — raw byte management, content addressing, deduplication. Delegated to the community-maintained [`holochain-open-dev/file-storage`](https://github.com/holochain-open-dev/file-storage) zome.

2. **File system organization** — hierarchical paths, author attribution, versioning, directory traversal. Owned by this project.

This separation means community maintenance of the chunk layer (HDK version updates, optimizations, security fixes) automatically benefits this project without requiring changes to the FS logic.

---

## Layer Architecture

```
╔════════════════════════════════════════════════════════════════╗
║  Application Layer (Nondominium / hAppenings / IDI)           ║
║  Calls create_file(), get_files_by_path(), update_file()      ║
╚══════════════════════════════╦═════════════════════════════════╝
                               │
╔══════════════════════════════▼═════════════════════════════════╗
║  File System Layer (this project)                              ║
║                                                                ║
║  ┌────────────────────────┐  ┌──────────────────────────────┐  ║
║  │   Integrity Zome       │  │   Coordinator Zome           │  ║
║  │ ────────────────────── │  │ ─────────────────────────── │  ║
║  │ FileMetadata entry     │  │ create_file()                │  ║
║  │ PathFileSystem link    │  │ update_file()                │  ║
║  │ PathToFileMetaData lnk │  │ delete_file()                │  ║
║  │ FileMetaDataUpdate lnk │  │ get_file_metadata()          │  ║
║  │ Validation rules       │  │ get_file_chunks()            │  ║
║  └────────────────────────┘  │ get_files_by_path()          │  ║
║                              │ Path conversion utilities    │  ║
║                              │ post_commit signals          │  ║
║                              └──────────────────────────────┘  ║
╚══════════════════════════════╦═════════════════════════════════╝
                               │ call() — chunk operations
╔══════════════════════════════▼═════════════════════════════════╗
║  Community Chunk Layer (holochain-open-dev/file-storage)       ║
║                                                                ║
║  ┌────────────────────────┐  ┌──────────────────────────────┐  ║
║  │   Integrity Zome       │  │   Coordinator Zome           │  ║
║  │ ────────────────────── │  │ ─────────────────────────── │  ║
║  │ FileChunk entry        │  │ create_file_chunk()          │  ║
║  │                        │  │ get_file_chunk()             │  ║
║  └────────────────────────┘  └──────────────────────────────┘  ║
╚════════════════════════════════════════════════════════════════╝
```

---

## Entry Types

### `FileMetadata` (FS Layer — Integrity Zome)

The central organizing entry. One per file, updated on version changes.

```rust
pub struct FileMetadata {
    pub name: String,           // File name (immutable after creation)
    pub author: AgentPubKey,    // Original creator (immutable)
    pub path: String,           // Filesystem path, e.g. "/documents/report.pdf"
    pub created: Timestamp,     // Creation time (immutable)
    pub last_modified: Timestamp, // Updated on each version
    pub size: usize,            // Total size in bytes (latest version)
    pub file_type: String,      // MIME type or extension (immutable)
    pub chunks_hashes: Vec<EntryHash>, // Ordered chunk entry hashes (latest version)
}
```

**Immutable fields:** `name`, `author`, `path`, `created`, `file_type` — these never change across versions.
**Mutable fields:** `last_modified`, `size`, `chunks_hashes` — updated on each version.

### `FileChunk` (Community Layer — Integrity Zome)

A raw byte array representing one segment of a file, up to 1 MB.

```rust
pub struct FileChunk(pub SerializedBytes);
```

Chunks are content-addressed: identical byte sequences across different files produce the same entry hash and are stored only once on the DHT.

---

## Link Types

### `PathFileSystem`

Typed path entries using Holochain's native `Path` system. Forms the directory tree.

```
Path("root")
  └── Path("root.documents")
       └── Path("root.documents.reports")
```

Each path node is created via `typed_path.ensure()` when a file is first stored at that location.

### `PathToFileMetaData`

Links a path entry hash to a `FileMetadata` action hash. Enables directory listing.

```
Path("root.documents") ──PathToFileMetaData──> FileMetadata action hash
```

Created when a file is stored. **Note:** must also be deleted when the file is deleted (current known bug).

### `FileMetaDataUpdate`

Links the original `FileMetadata` action hash to the updated version's action hash. Forms the version chain.

```
[Original ActionHash] ──FileMetaDataUpdate──> [v2 ActionHash] ──> [v3 ActionHash]
```

`get_file_metadata()` traverses this chain to find the most recent version by timestamp.

---

## Path System

### Conversion

Filesystem paths use `/` separator. DHT paths use `.` separator with `root` prefix.

```
/documents/reports      →  root.documents.reports
/                       →  root
documents/reports       →  root.documents.reports  (leading slash added)
/documents//reports/    →  root.documents.reports  (normalized)
```

The conversion is handled by `fs_path_to_dht_path()` and `standardize_fs_path()` in `files.rs`.

### Forbidden Characters

Path segments must not contain: `< > : " | ? * .`

The `.` character is forbidden because it is used as the DHT path separator — a segment containing `.` would corrupt the path tree.

Validation is enforced in the integrity zome's `validate()` function, making it a network-wide rule.

### Directory Traversal

`get_files_metadata_by_path_recursively()` uses Holochain's `children_paths()` on typed paths to enumerate all subdirectories, then collects files from each level.

```
get_files("/documents")
  → files at root.documents
  → get_files("/documents/reports")    (child path)
       → files at root.documents.reports
       → get_files("/documents/reports/2024")  (child path)
            → files at root.documents.reports.2024
```

---

## Versioning Model

### Update Flow

When `update_file()` is called:

1. Retrieve current `FileMetadata` (following any existing version chain to latest)
2. Create new chunks for the new content via chunk layer
3. Create a new `FileMetadata` entry with updated `last_modified`, `size`, `chunks_hashes`
4. Create a `FileMetaDataUpdate` link: original hash → new hash
5. Mark old chunks as deleted (they are superseded)
6. Return the new `FileOutput`

### Resolution Flow

When `get_file_metadata(original_hash)` is called:

1. Query all `FileMetaDataUpdate` links from `original_hash`
2. Select the link with the most recent timestamp
3. If no links exist: return the original entry
4. If links exist: return the entry at the latest link target

### Stable Identity

The original action hash is the stable, shareable identifier for a file. Applications that store references to files (e.g., a governance entry linking to an evidence document) use the original hash. The version resolution happens transparently at read time.

---

## Signal Architecture

Signals are emitted from the `post_commit` hook (not from coordinator functions). This guarantees signals are only emitted for actions that have been committed and validated by the DHT.

```rust
pub enum Signal {
    FileMetadataCreated {
        action: SignedActionHashed,
        app_entry: EntryTypes,
    },
    FileMetadataUpdated {
        action: SignedActionHashed,
        app_entry: EntryTypes,       // New version
        original_app_entry: EntryTypes, // Previous version
    },
    FileMetadataDeleted {
        action: SignedActionHashed,
        original_app_entry: EntryTypes,
    },
}
```

Only `FileMetadata` actions emit signals — `FileChunk` actions are transparent to UI clients.

---

## Cross-DNA Architecture (Technology Trinity)

The Technology Trinity (Nondominium + hAppenings + IDI) can share a single file storage provider DNA:

```
┌──────────────────────┐  ┌──────────────────────┐  ┌──────────────────────┐
│   Nondominium DNA    │  │   hAppenings DNA      │  │      IDI DNA         │
│ ─────────────────── │  │ ─────────────────────│  │ ──────────────────── │
│ file_system (coord) │  │ file_system (coord)   │  │ file_system (coord)  │
│   ↓ call()          │  │   ↓ call()            │  │   ↓ call()           │
└──────────┬───────────┘  └──────────┬────────────┘  └──────────┬───────────┘
           │                         │                           │
           └─────────────────────────┼───────────────────────────┘
                                     │
                        ┌────────────▼────────────┐
                        │  File Storage Provider  │
                        │  DNA (community zome)   │
                        │  FileChunk storage      │
                        └─────────────────────────┘
```

In this model:
- Each application DNA embeds the File System coordinator/integrity zome pair
- Each application maintains its own `FileMetadata` entries and path namespaces on its own DHT
- All three delegate chunk storage to the shared provider DNA
- Chunks are deduplicated across all three applications automatically

Alternatively, each application can embed both layers in its own DNA (no sharing). This is simpler to deploy and appropriate when cross-application file deduplication is not a priority.

---

## Known Issues (Current Implementation)

| Issue | Location | Impact | Fix |
|-------|----------|--------|-----|
| Path links not cleaned up on delete | `coordinator/lib.rs:delete_file()` | `PathToFileMetaData` links remain after deletion; deleted files still appear in directory listings | Delete the link in `delete_file()` |
| Debug `println!` in path conversion | `coordinator/files.rs:standardize_fs_path()` | Noisy WASM output | Remove the `println!` |
| HDK v0.1.2 / HDI v0.2.2 | `Cargo.toml` | Cannot compile against current Holochain | Update to current stable |
| No file move/rename | — | Cannot reorganize files without delete+recreate | Add `move_file()` function |
