# Requirements — File System Zome

## Context

This zome is infrastructure for the **Technology Trinity**: three Holochain applications that together form the technical foundation of the commons.

### Technology Trinity

| Application | Purpose | File Storage Need |
|-------------|---------|-------------------|
| **Nondominium** | Commons governance, resource tracking, shared asset management | Governance proposals, resource documentation, evidence attachments organized by resource and community |
| **hAppenings** | Requests and offers coordination | Images and attachments for offers and requests |
| **IDI** (Intelligent Didactic Interface) | Non-linear learning paths, learner profiles | Course modules, learning content, resources organized by curriculum path |

Each application requires hierarchical file organization. They may share a single file storage provider DNA via Holochain's cell bridge pattern — preventing data duplication and making file storage a true commons resource.

---

## Functional Requirements

### FR-1: Hierarchical Path Organization

The zome must support organizing files into a navigable directory hierarchy.

- Files are addressed by a filesystem-style path (e.g., `/governance/proposals/`, `/courses/rust/module-1/`)
- Directories are implicit — they exist when a file exists within them
- Path segments must not contain forbidden characters: `< > : " | ? * .`
- The path system must support recursive listing (all files under a path and its subdirectories)
- The root path `/` is valid and represents the top-level namespace

**Rationale:** IDI learning content must be navigable by course/module/lesson. Nondominium documents must be organized by resource and governance process. Without paths, all files are a flat blob store with no meaningful organization.

### FR-2: Author Attribution

Every file must record the `AgentPubKey` of its original creator.

- Author is set at creation time and is immutable
- Author reflects the Holochain agent who called `create_file`, not a user-supplied value
- Attribution is preserved through version updates

**Rationale:** Commons governance requires knowing who created or uploaded a document. Nondominium resource governance depends on attributed authorship.

### FR-3: Content Versioning

File content must support update without destroying history.

- A file update creates a new version linked to the original via a version chain
- `get_file_metadata()` always resolves to the latest version
- Old chunks are cleaned up (marked deleted) when a new version supersedes them
- The original file action hash remains the stable identifier across all versions

**Rationale:** Governance documents change over time. Learning content is revised. The stable original hash allows references (links from governance entries, bookmarks, etc.) to survive updates without breaking.

### FR-4: File Chunking

Large files must be split into content-addressed chunks for DHT storage.

- Maximum chunk size: 1 MB
- Chunks are content-addressed (identical chunks across files deduplicate automatically)
- Chunks are stored and retrieved in order
- The full file content is reassembled from the ordered chunk list in `FileMetadata.chunks_hashes`

**Rationale:** Holochain DHT entries have size limits. Chunking allows arbitrarily large files while keeping individual entries small enough for efficient DHT propagation.

### FR-5: Real-Time Signals

The zome must emit signals to connected UI clients after every file operation.

- `FileMetadataCreated` — emitted when a new file is created
- `FileMetadataUpdated` — emitted when a file is updated (includes before/after metadata)
- `FileMetadataDeleted` — emitted when a file is deleted

**Rationale:** UI clients need to update their views reactively when other agents create, update, or delete files without requiring polling.

### FR-6: File Uniqueness Enforcement

The zome must prevent creating two files with the same name at the same path.

- Attempting to create a file at a path/name combination that already exists must fail with a clear error
- This check is best-effort (DHT eventual consistency may allow edge-case races, but the normal path must enforce uniqueness)

### FR-7: Cross-DNA Compatibility

The zome must be embeddable in other DNAs without modification.

- The zome is a coordinator/integrity pair that can be included in any `dna.yaml`
- No hardcoded DNA-specific assumptions
- The chunk layer is callable via Holochain's `call()` mechanism, supporting both same-DNA and cross-DNA chunk storage

---

## Non-Functional Requirements

### NFR-1: Modern HDK Compatibility

The zome must target Holochain 0.4.x (HDK/HDI current stable).

- The current implementation uses HDI v0.2.2 / HDK v0.1.2 and must be updated
- The updated zome must pass all existing tests against the new HDK version

### NFR-2: Community Zome Integration

Chunk operations must delegate to `holochain-open-dev/file-storage`.

- This allows the chunk layer to receive community maintenance (HDK updates, security fixes)
- The deduplication benefit (`create_relaxed` semantics) is preserved
- The FS layer (this project) owns only `FileMetadata` and the path/versioning logic

### NFR-3: Test Coverage

All public functions must have integration tests using `@holochain/tryorama`.

- Tests run two agents (Alice and Bob) on a shared DHT
- Coverage must include: create, update, delete, path traversal, version chain, signals
- Known edge cases: empty file name, forbidden path characters, concurrent agents

---

## Out of Scope

- File permissions or access control (handled at the application layer by consuming DNAs)
- Full version history retrieval (only latest version is surfaced; prior versions are cleaned up)
- Binary diff storage (full chunk replacement on every update)
- Directory creation as an explicit operation (directories are implicit from file paths)
