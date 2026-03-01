# File System Zome

A reusable Holochain zome that provides a distributed, hierarchical file system layer for Holochain applications. It organizes files into navigable paths, tracks authorship, supports content versioning, and emits real-time signals — built as part of the [Technology Trinity](documentation/REQUIREMENTS.md#technology-trinity) infrastructure for the commons.

## New Direction

This project is evolving into a **two-layer architecture**:

1. **Chunk storage layer** — delegated to the community-maintained [`holochain-open-dev/file-storage`](https://github.com/holochain-open-dev/file-storage) zome. It handles raw byte chunking, deduplication, and blob retrieval using modern HDK.

2. **File system layer** (this project) — adds the organizing intelligence: hierarchical paths, author attribution, content versioning, directory traversal, and signals. This is what makes a blob store into a navigable file system.

This split allows Nondominium, hAppenings, and IDI — the three systems of the Technology Trinity — to share a single file storage provider DNA while each maintains its own path-organized file namespaces.

See [`documentation/ARCHITECTURE.md`](documentation/ARCHITECTURE.md) for the full design and [`documentation/SPECIFICATION.md`](documentation/SPECIFICATION.md) for the implementation roadmap.

## Current State

The current implementation is a proof-of-concept built with HDI v0.2.2 / HDK v0.1.2. It demonstrates the full path/versioning model but needs to be updated to modern HDK and refactored to delegate chunk operations to the community zome. The core data model and algorithms are sound and carry forward into the new architecture.

**What works today:**
- Hierarchical path system (`/subfolder/file` → `root.subfolder` DHT paths)
- File chunking (1 MB chunks, content-addressed)
- Version chain (`FileMetaDataUpdate` links)
- Recursive directory traversal
- Post-commit signals (created/updated/deleted)

**What needs work:**
- HDK/HDI version update (target: Holochain 0.4.x)
- Delegate chunk operations to community zome
- Fix path link cleanup on file deletion
- Add file move/rename capability

## Architecture

### Two-Layer Design

```
┌──────────────────────────────────────────────────────┐
│  File System Layer (this project)                    │
│  ┌──────────────────┐  ┌─────────────────────────┐   │
│  │ Integrity Zome   │  │ Coordinator Zome        │   │
│  │ ───────────────  │  │ ──────────────────────  │   │
│  │ FileMetadata     │  │ create_file()           │   │
│  │  - name          │  │ update_file()           │   │
│  │  - author        │  │ delete_file()           │   │
│  │  - path          │  │ get_files_by_path()     │   │
│  │  - created       │  │ Path system + signals   │   │
│  │  - last_modified │  └─────────────────────────┘   │
│  │  - size          │                                │
│  │  - file_type     │                                │
│  │  - chunks_hashes │                                │
│  └──────────────────┘                                │
└──────────────────────────────────────────────────────┘
                        │ delegates chunk ops via call()
┌─────────────────────────────────────────────────────┐
│  Community Chunk Layer (holochain-open-dev)         │
│  FileChunk storage, deduplication, blob retrieval   │
│  Maintained by the community — HDK always current   │
└─────────────────────────────────────────────────────┘
```

### Entry Types

**`FileMetadata`** — stores organizing information about a file:

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | File name |
| `author` | `AgentPubKey` | Original creator's public key |
| `path` | `String` | Filesystem path (e.g., `/documents/report.pdf`) |
| `created` | `Timestamp` | Creation time |
| `last_modified` | `Timestamp` | Last update time |
| `size` | `usize` | Total file size in bytes |
| `file_type` | `String` | MIME type or extension |
| `chunks_hashes` | `Vec<EntryHash>` | Ordered chunk entry hashes |

**`FileChunk`** (community zome) — raw byte array up to 1 MB.

### Link Types

| Link | Source | Target | Purpose |
|------|--------|--------|---------|
| `PathFileSystem` | Path entry | Path entry | DHT path tree for directory hierarchy |
| `PathToFileMetaData` | Path entry | FileMetadata action hash | Directory → file index |
| `FileMetaDataUpdate` | Original metadata hash | Updated metadata hash | Version chain |

### Path System

Filesystem paths are converted to Holochain's native `Path` system:

```
/documents/reports/q1.pdf  →  root.documents.reports
```

- Separator: `/` in filesystem paths, `.` in DHT paths
- Root prefix: all paths start from `root`
- Forbidden characters in path segments: `< > : " | ? * .`

### Versioning Model

File updates create a linked version chain rather than mutating entries:

```
[Original FileMetadata] ──FileMetaDataUpdate──> [v2 FileMetadata] ──> [v3 FileMetadata]
```

`get_file_metadata()` always resolves to the latest version by following the chain.
Old chunks are marked deleted when a new version is created.

### Signals

Emitted via `post_commit` after every committed action on `FileMetadata` entries:

- `FileMetadataCreated` — new file created
- `FileMetadataUpdated` — file content or metadata updated
- `FileMetadataDeleted` — file and its version chain deleted

## Public API

### `create_file(FileInput) → FileOutput`

Creates a new file at the given path.

```rust
pub struct FileInput {
    pub name: String,
    pub path: String,
    pub file_type: String,
    pub content: SerializedBytes,
}
```

Fails if a file with the same name already exists at that path.

### `update_file(UpdateFileMetadataInput) → FileOutput`

Updates file content, creating a new version in the chain.

```rust
pub struct UpdateFileMetadataInput {
    pub original_file_metadata_hash: ActionHash,
    pub new_content: SerializedBytes,
}
```

### `delete_file(ActionHash) → Vec<ActionHash>`

Deletes a file and all its versioned chunks.

### `get_file_metadata(ActionHash) → Option<Record>`

Returns the latest version of a file's metadata.

### `get_file_chunks(ActionHash) → Vec<Record>`

Returns all chunks for the latest version of a file.

### `get_files_metadata_by_path_recursively(String) → Vec<Record>`

Lists all files under a path and its subdirectories.

```
get_files_metadata_by_path_recursively("/documents")
// returns files in /documents, /documents/reports, /documents/reports/2024, etc.
```

## Integration

This zome is designed to be embedded in other Holochain DNAs. Add it to your `dna.yaml` alongside your application zomes.

See [`documentation/REQUIREMENTS.md`](documentation/REQUIREMENTS.md) for the Great Work integration context and [`documentation/SPECIFICATION.md`](documentation/SPECIFICATION.md) for the implementation roadmap.

## Development

All commands must be run from inside the Nix shell (`nix-shell` in the root directory).

```bash
# Install dependencies
npm install

# Build Rust WASM zomes
npm run build:zomes

# Build the full hApp
npm run build:happ

# Run all tests
npm test

# Run a single test
cd tests && npx vitest run --reporter=verbose -t "create file with empty name"

# Start a 2-agent network
npm start
```

## License and Attribution

Inspired by the [file-storage zome](https://github.com/holochain-open-dev/file-storage) from [Holochain Open Dev](https://holochain-open-dev.github.io/). The chunk storage layer is delegated back to that community project in the new architecture.
