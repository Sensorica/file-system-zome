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

Every file must record its original creator via an `AuthorToFileMetadata` DHT link, not as a field in the `FileMetadata` entry.

- An `AuthorToFileMetadata` link is created from the author's identity hash to the `FileMetadata` action hash at file creation time
- The link base is `AnyLinkableHash`: typically an `AgentPubKey`, but consuming hApps may substitute a profile entry hash or other identity anchor
- The link is never deleted — authorship is permanent and immutable
- The author is determined by the calling agent at `create_file` time, not a user-supplied value
- Attribution is preserved through version updates: the link targets the original action hash, which is the stable file identifier

**Rationale:** Commons governance requires knowing who created or uploaded a document. Nondominium resource governance depends on attributed authorship. Using a flexible link base allows consuming hApps (with their own profile/identity systems) to attribute files to a richer identity object rather than a raw agent key.

### FR-3: Content Versioning

File content must support update without destroying history.

- A file update creates a new version linked to the original via a version chain
- `get_file_metadata()` always resolves to the latest version
- Old chunks are cleaned up (marked deleted) when a new version supersedes them
- The original file action hash remains the stable identifier across all versions
- File creation and modification times are available from Holochain action metadata (`action.timestamp()`), not from entry fields

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

The zome must target Holochain 0.6.x (HDK/HDI current stable).

- The current implementation uses HDI v0.2.2 / HDK v0.1.2 and must be updated
- The updated zome must pass all existing tests against the new HDK version

### NFR-2: Community Zome Integration

Chunk operations must delegate to `holochain-open-dev/file-storage`.

- This allows the chunk layer to receive community maintenance (HDK updates, security fixes)
- The deduplication benefit (`create_relaxed` semantics) is preserved
- The FS layer (this project) owns only `FileMetadata` and the path/versioning logic

### NFR-3: Test Coverage

Public functions must be tested at two levels using complementary frameworks.

**Zome-level tests (sweettest):** Rust-native in-process tests using `holochain::sweettest`.

- Fast feedback loop: tests run in-process with no WASM compile step per test
- Full Rust type safety — no type redefinition needed between zome and test code
- Use for: validation rules, entry creation/update/delete logic, version chain traversal, path conversion
- Located in `dnas/file_system/zomes/coordinator/file_system/tests/` (Rust `#[tokio::test]` functions)

**Scenario-level tests (tryorama):** TypeScript integration tests using `@holochain/tryorama`.

- Tests run two agents (Alice and Bob) against a real conductor over WebSocket
- Coverage must include: create, update, delete, path traversal, version chain, signals
- Known edge cases: empty file name, forbidden path characters, concurrent agents
- Located in `tests/` (TypeScript with vitest)

Both test suites must pass before any phase is considered complete.

---

## Out of Scope

- File permissions or access control (handled at the application layer by consuming DNAs)
- Full version history retrieval (only latest version is surfaced; prior versions are cleaned up)
- Binary diff storage (full chunk replacement on every update)
- Directory creation as an explicit operation (directories are implicit from file paths)
