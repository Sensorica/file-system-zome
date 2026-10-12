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
║  Application Layer (Nondominium / hAppenings / IDI)            ║
║  Calls create_file(), get_files_by_path(), update_file()       ║
╚══════════════════════════════╦═════════════════════════════════╝
                               ║
                               ▼
╔═════════════════════════════════════════════════════════════════╗
║  File System Layer (this project)                               ║
║                                                                 ║
║  ┌─────────────────────────┐  ┌──────────────────────────────┐  ║
║  │   Integrity Zome        │  │   Coordinator Zome           │  ║
║  │ ──────────────────────  │  │ ──────────────────────────── │  ║
║  │ FileMetadata entry      │  │ create_file()                │  ║
║  │ PathFileSystem link     │  │ update_file()                │  ║
║  │ PathToFileMetaData lnk  │  │ delete_file()                │  ║
║  │ FileMetaDataUpdate lnk  │  │ get_file_metadata()          │  ║
║  │ AuthorToFileMetaData lnk│  │ get_file_chunks()            │  ║
║  │ Validation rules        │  │ get_files_by_path()          │  ║
║  └─────────────────────────┘  │ Path conversion utilities    │  ║
║                               │ post_commit signals          │  ║
║                               └──────────────────────────────┘  ║
╚══════════════════════════════╦══════════════════════════════════╝
                               │ call() — chunk operations
                               ▼
╔════════════════════════════════════════════════════════════════╗
║  Community Chunk Layer (holochain-open-dev/file-storage)       ║
║                                                                ║
║  ┌────────────────────────┐  ┌──────────────────────────────┐  ║
║  │   Integrity Zome       │  │   Coordinator Zome           │  ║
║  │ ────────────────────── │  │ ──────────────────────────── │  ║
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
    pub path: String,           // Filesystem path, e.g. "/documents/report.pdf"
    pub size: usize,            // Total size in bytes (latest version)
    pub file_type: String,      // MIME type or extension (immutable)
    pub chunks_hashes: Vec<EntryHash>, // Ordered chunk entry hashes (latest version)
}
```

**Immutable fields:** `name`, `path`, `file_type` — these never change across versions.
**Mutable fields:** `size`, `chunks_hashes` — updated on each version.

> **Timestamps** are not stored in the entry — `created` is the timestamp of the create action, and `last_modified` is the timestamp of the latest update action. Both are available from Holochain action metadata.
>
> **Authorship** is tracked via the `AuthorToFileMetadata` link (see Link Types below), not as an entry field. The link base can be an `AgentPubKey` or any identity anchor (e.g., a profile entry hash) defined by the consuming hApp.

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

### `AuthorToFileMetadata`

Links an author identity anchor to the `FileMetadata` action hash. Records authorship on the DHT.

```
AnyLinkableHash (author) ──AuthorToFileMetadata──> FileMetadata action hash
```

- **Base:** Any linkable hash representing the author — typically an `AgentPubKey`, but can be a profile entry hash or other identity anchor defined by the consuming hApp.
- **Target:** The `FileMetadata` action hash (original, stable identifier).
- Created at file creation time. Never deleted — authorship is permanent.
- Enables querying all files created by a given identity via `get_links(author_hash, AuthorToFileMetadata)`.

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
3. Create a new `FileMetadata` entry with updated `size`, `chunks_hashes`
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

## Testing Architecture

Two frameworks serve distinct roles in the test pyramid.

### Sweettest — Rust In-Process Tests

`holochain::sweettest` is a module inside the `holochain` crate (not a separate dependency). It creates conductors in-memory, installs DNA files, and calls zome functions directly from Rust.

```
┌─────────────────────────────────────────────────────┐
│  Test Process (cargo test)                          │
│                                                     │
│  SweetConductor (in-memory conductor)               │
│    └── SweetApp                                     │
│          └── SweetCell (agent + DNA pair)           │
│                └── call(&cell.zome("name"), fn, ...) │
└─────────────────────────────────────────────────────┘
```

**Key types:**
- `SweetConductor` — in-memory conductor, created per test or shared across tests
- `SweetDnaFile` — wraps compiled WASM into a DNA for installation
- `SweetApp` — installed hApp within a conductor
- `SweetCell` — cell instance for one agent+DNA pair; used as the target for `call()`
- `SweetAgents` — manages agent keypairs for multi-agent tests

**Strengths:** Full Rust type safety, fast execution, no external process, inline zomes supported for isolated unit tests. Direct access to validation errors as Rust types rather than serialized JSON.

**Limitation:** Cannot test real WebSocket signal delivery to a TypeScript client.

Multi-agent scenarios run on a `SweetConductorBatch` (one conductor per agent) with `await_consistency` before cross-agent reads, which is how the current suite exercises DHT propagation between Alice and Bob.

### Client-level tests (planned)

`@holochain/tryorama` filled this role on Holochain 0.1 but has no 0.7 release. The replacement is a TypeScript suite using `@holochain/client` against conductors started with `hc sandbox` (as `npm start` does).

```
┌─────────────────────┐   WebSocket (client)  ┌──────────────────────┐
│  Test Process       │ ◄──────────────────── │  Holochain Conductor  │
│  (vitest / Node)    │                       │  (real process)       │
│                     │                       │  File system hApp     │
│  Alice cell         │                       │  installed            │
│  Bob cell           │                       │                       │
└─────────────────────┘                       └──────────────────────┘
```

**Strengths:** Tests the full stack including conductor, signal emission, and multi-agent DHT gossip. The test environment mirrors real production usage from a TypeScript frontend.

**Limitation:** Requires a compiled `.happ` file before tests run; slower than sweettest for tight Rust-only loops.

### When to Use Each

| Scenario | Framework |
|----------|-----------|
| Validation rule logic | sweettest |
| Entry CRUD operations | sweettest |
| Path conversion utilities | sweettest |
| Version chain traversal | sweettest |
| Signal emission (real conductor) | client-level (planned) |
| Multi-agent DHT behavior | sweettest (two conductors) |
| TypeScript client integration | client-level (planned) |
| Fast CI feedback on Rust changes | sweettest |
| End-to-end scenario tests | client-level (planned) |

Both levels test the same compiled WASM: sweettest loads it in-process, the client level loads it into a running conductor.

---

## Known Issues (Current Implementation)

| Issue | Location | Impact | Fix |
|-------|----------|--------|-----|
| Shared chunks | `coordinator/files.rs:create_file_chunk()` | Chunks are deduplicated by content; deleting or updating one file deletes a chunk another file with identical content still references | Phase 2 (community chunk layer) or per-file chunk ownership |
| No ownership rules | `integrity/lib.rs:validate()` | Any agent can update or delete any file; `name`, `path`, `file_type` are not enforced as immutable | Author check and field immutability in validation |
| No file move/rename | n/a | Cannot reorganize files without delete+recreate | Add `move_file()` function |

Fixed on 2026-10-12 (see `CHANGELOG.md`): path links left behind on delete, debug `println!` in path conversion, chunks of never-updated files left behind on delete, and the HDK v0.1.2 / HDI v0.2.2 pin.
