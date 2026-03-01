# File System Zome

A reusable Holochain zome for storing and managing files in a distributed file-system-like structure on the Holochain network. This zome can be integrated into any Holochain application requiring file storage capabilities.

## DEPRECATION NOTICE

This zome was built with **HDI v0.2.2** and **HDK v0.1.2**. Holochain has evolved significantly since these versions were released. Before using this zome in production, be aware that:

- The HDK and HDI versions are outdated
- There may be breaking changes in newer Holochain releases
- Security, performance, and API improvements have been made in later versions
- Consider evaluating this against current Holochain best practices

This repository serves as a reference implementation and proof-of-concept for file storage on Holochain.

## Architecture

### Zome Composition

The File System Zome is composed of two distinct zome types following modern Holochain architecture:

- **Integrity Zome**: Defines the entry types, link types, and validation rules for file system operations. It ensures data consistency and integrity across the distributed network.
- **Coordinator Zome**: Implements the public-facing API and business logic for file operations. It coordinates with the integrity zome to perform validated file operations.

This separation of concerns provides better validation control and allows for safer zome updates.

### File Storage Model

The zome stores files and their metadata using a chunking system:

- Files are automatically split into **1 MB chunks** for efficient storage and retrieval
- Each chunk is stored as a `FileChunk` entry on the DHT
- File metadata is stored as a `FileMetadata` entry that references all associated chunks

### Path System

Paths in the file system use Holochain's native path system with custom transformation:

- Filesystem paths use forward slashes (e.g., `/subfolder/file`)
- Paths are converted to DHT notation using dots as separators with a `root` prefix
- Example transformation: `/subfolder/file` → `root.subfolder.file`
- **Forbidden characters** in paths: `< > : " | ? *` and `.`
  - These characters cannot be used in file or folder names to ensure compatibility with the path system

### Versioning and Updates

The zome implements a versioning model based on entry chaining:

- When a file is **updated**, a new `FileMetadata` entry is created with the updated content
- The new entry is linked to the original via a `FileMetaDataUpdate` link, forming a version chain
- Old file chunks are marked as deleted on the DHT
- **File recovery is not supported**: Once a new version is created or a file is deleted, previous versions cannot be restored
- The zome always returns the latest version when retrieving files

### Entry Definitions

#### FileMetadata
Stores comprehensive metadata about a file:
- `name`: File name
- `path`: File path in the file system (filesystem format with `/`)
- `author`: `AgentPubKey` of the original creator
- `created`: Creation timestamp
- `last_modified`: Last modification timestamp
- `file_type`: MIME type or file extension
- `size`: Total file size in bytes
- `chunks_hashes`: List of `EntryHash` values for all `FileChunk` entries that compose the file

#### FileChunk
Stores a portion of file content:
- `content`: Serialized byte array representing up to 1 MB of file data
- Referenced by `FileMetadata` entries

### Link Types

- **PathFileSystem**: Typed path entry for organizing files hierarchically in the file system structure
- **PathToFileMetaData**: Links a path entry to the original `FileMetadata` entry, enabling path-based file discovery
- **FileMetaDataUpdate**: Links updated `FileMetadata` entries to their previous versions, creating an immutable version chain

### Public Functions

#### create_file(file_input: FileInput) -> ExternResult<FileOutput>
Creates a new file in the file system.

**Parameters:**
- `file_input`: Contains the file's name, path, type, and binary content

**Behavior:**
- Validates that the file does not already exist at the specified path
- Splits the file content into 1 MB chunks
- Creates `FileChunk` entries for each chunk
- Creates a `FileMetadata` entry linking all chunks
- Establishes path links for hierarchical access

**Returns:** `FileOutput` containing the metadata entry and all chunk entries

#### get_file_chunks(file_metadata_hash: ActionHash) -> ExternResult<Vec<Record>>
Retrieves all file chunks for a given file.

**Parameters:**
- `file_metadata_hash`: The action hash of a `FileMetadata` entry

**Returns:** Vector of `Record` entries containing the file chunks in order

#### get_file_metadata(original_file_metadata_hash: ActionHash) -> ExternResult<Option<Record>>
Retrieves the latest version of a file's metadata.

**Parameters:**
- `original_file_metadata_hash`: The hash of the original `FileMetadata` entry

**Returns:** `Option<Record>` containing the latest metadata entry, or `None` if not found

#### get_files_metadata_by_path_recursively(path_string: String) -> ExternResult<Vec<Record>>
Retrieves all files under a given directory path and its subdirectories.

**Parameters:**
- `path_string`: The directory path in filesystem format (e.g., `/documents/reports`)

**Returns:** Vector of `Record` entries for all files found recursively

#### update_file(update_file_metadata_input: UpdateFileMetadataInput) -> ExternResult<FileOutput>
Updates an existing file with new content and/or metadata.

**Parameters:**
- `update_file_metadata_input`: Contains the file metadata hash, new name, new path, new type, and new content

**Behavior:**
- Creates a new `FileMetadata` entry with updated information
- Creates new `FileChunk` entries for the updated content
- Links the new metadata to the original via `FileMetaDataUpdate` to form a version chain
- Marks old chunks as deleted
- Updates path links

**Returns:** `FileOutput` containing the new metadata entry and chunk entries

#### delete_file(original_file_metadata_hash: ActionHash) -> ExternResult<Vec<ActionHash>>
Deletes a file from the file system.

**Parameters:**
- `original_file_metadata_hash`: The hash of the original `FileMetadata` entry

**Behavior:**
- Marks the file metadata entry as deleted
- Marks all associated file chunks as deleted
- Removes path links

**Returns:** Vector of all entry hashes that were marked as deleted

### Signals

The zome emits the following signals to notify clients of file system changes:

- **FileMetadataCreated**: Emitted when a new file is created. Contains the metadata entry details.
- **FileMetadataUpdated**: Emitted when a file is updated. Contains the new metadata entry and version chain information.
- **FileMetadataDeleted**: Emitted when a file is deleted. Contains the deleted metadata entry hash.

Clients can subscribe to these signals to stay synchronized with file system changes across the network.

## Data Types

### FileInput
Input structure for file creation:
```rust
pub struct FileInput {
    pub name: String,                // File name
    pub path: String,                // File path (filesystem format with /)
    pub file_type: String,           // File type/MIME type
    pub content: SerializedBytes,    // Raw file content as serialized bytes
}
```

### FileOutput
Output structure returned from file operations:
```rust
pub struct FileOutput {
    pub file_metadata: Record,       // The FileMetadata entry
    pub file_chunks: Vec<Record>,    // All FileChunk entries
}
```

### UpdateFileMetadataInput
Input structure for file updates. Note: only content can be updated — name, path, and type are immutable after creation:
```rust
pub struct UpdateFileMetadataInput {
    pub original_file_metadata_hash: ActionHash,  // Hash of original metadata
    pub new_content: SerializedBytes,              // Updated file content
}
```

## Environment Setup

### Prerequisites

Set up the [Holochain development environment](https://developer.holochain.org/docs/install/).

### Installation

Enter the nix shell and install dependencies:

```bash
nix-shell
npm install
```

**Important:** Run all subsequent commands from within the nix-shell. The Holochain tools and dependencies are only available inside the nix environment.

## Running Tests

Run the test suite:

```bash
npm test
```

This executes the Tryorama test suite to validate zome functionality.

## Building

Build the zome and hApp:

```bash
npm run build:zomes
npm run build:happ
```

## Running 2 Agents

Start a development network with 2 connected agents:

```bash
npm start
```

This will:
- Create a network of 2 Holochain nodes connected to each other
- Launch UIs for each agent
- Bring up the Holochain Playground for advanced conductor introspection

## Bootstrapping a Multi-Agent Network

Create a custom network with multiple agents:

```bash
AGENTS=3 npm run network
```

Replace "3" with the desired number of agents. This also brings up the Holochain Playground for monitoring.

## Packaging

Package the zome and hApp into a distributable webhapp:

```bash
npm run package
```

Output files:
- `test-happ.webhapp`: Complete web hApp bundle for distribution via Holochain Launcher
- `test-happ.happ`: The hApp package without web UI

## Development Tools

This repository utilizes:

- **[NPM Workspaces](https://docs.npmjs.com/cli/v7/using-npm/workspaces/)**: Monorepo management with npm v7+
- **[hc CLI](https://github.com/holochain/holochain/tree/develop/crates/hc)**: Holochain CLI for development instance management
- **[@holochain/tryorama](https://www.npmjs.com/package/@holochain/tryorama)**: Test framework for Holochain zomes
- **[@holochain/client](https://www.npmjs.com/package/@holochain/client)**: Client library for UI-to-Holochain communication
- **[@holochain-playground/cli](https://www.npmjs.com/package/@holochain-playground/cli)**: Conductor introspection and debugging tools

## License and Attribution

Inspired by the [file-storage zome](https://github.com/holochain-open-dev/file-storage) from the [Holochain Open Dev](https://holochain-open-dev.github.io/) community.
