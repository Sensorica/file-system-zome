# Ideas to Explore

Future capabilities worth investigating for this file system zome. These are not committed to the roadmap — they are seeds for deeper thinking.

**Prerequisite:** All five ideas depend on the current refactor spec (Phases 1-5) completing first. None should be started before Phase 1 (HDK migration) is done.

**Design dependency order:** These ideas are not independent. The recommended implementation sequence is:
1. Relations Between Files (infrastructure — unblocks the others)
2. Template Files (builds on `DerivedFrom` relation)
3. Cryptographic Proof of Integrity — attestation subset (adds social layer)
4. File Manifests (builds on relations and integrity)
5. Commit History (reshapes the version chain — highest complexity)

---

## File Manifests

Group multiple files into a single declarative manifest (archive, package, governance record).

- A `FileManifest` entry type referencing an ordered list of `FileMetadata` action hashes
- Manifest-level metadata: name, description, author
- **Not atomic**: manifests use eventual consistency — files referenced may not have propagated to all peers yet
- `verify_completeness(manifest_hash) → {available: N, missing: M}` lets callers check availability
- Nested manifests: a manifest can reference other manifests (course → module manifests)
- Merkle root over all file hashes for bundle-level integrity verification (see Cryptographic Proof section)
- Use cases: IDI course packages, Nondominium governance record bundles

> **Design note**: Use the term "manifest" not "bundle" to avoid implying atomic semantics that cannot be guaranteed in an eventually-consistent DHT.

---

## Commit History

Enrich the current linear `FileMetaDataUpdate` chain with an explicit commit model.

**Tractable subset (implement this):**
- Explicit `Commit` entry type with: parent hash, author, timestamp, message, chunk list
- Tag support: named, immutable pointers to specific commit hashes
- Revert: create a new commit that restores a previous version's content
- Conflict detection: surface forks (two agents updating from the same base) to the application layer

**Defer for later:**
- Branching as a first-class concept — each agent's source chain is already a local branch; explicit branch types add complexity before the foundation is solid
- Merge commits — distributed merge of binary content has no clean solution without application-layer semantics
- Binary diff storage — full chunk replacement per version is simpler and correct

> **Design note**: Call it "Commit History" not "Git-Like Versioning" to set accurate expectations. Conflict *detection* is the zome's responsibility; conflict *resolution* is the consuming hApp's responsibility (e.g., Nondominium's governance process decides which version wins).

---

## Template Files

Predefined file structures that can be instantiated into new files.

- A `FileTemplate` entry type with parameterized content (placeholder tokens)
- A schema field alongside content defining variable names, types, and defaults (richer than bare token substitution)
- Instantiation: `instantiate_template(template_hash, variables) → FileMetadata` creates a new file
- Automatically creates a `DerivedFrom(template_entry_hash)` relation — records which exact version of the template was used
- Template versioning: templates use the same commit/update model as files
- Template library: templates are discoverable on the DHT, namespaced by author `AgentPubKey`
- Consuming apps curate which author namespaces they trust (governance of the shared library is a social problem, not a technical one)
- Use cases: IDI course module scaffolding, Nondominium governance motion forms, decision record templates

> **Design note**: Requires Relations to be implemented first. Without `DerivedFrom`, you lose the provenance link between instantiated files and their template version.

---

## Relations Between Files

Explicit typed relationships between file entries beyond parent/child paths.

- Relation types: `CopyOf`, `SymlinkTo`, `DerivedFrom`, `Supersedes`, `RelatedTo`
- Stored as typed links between `FileMetadata` action hashes
- `CopyOf`: marks a file as a duplicate of another, with original attribution
- `SymlinkTo`: a reference that resolves transparently to the target file on read
- `DerivedFrom`: tracks provenance (translated, processed, instantiated from template)
- `Supersedes`: explicit replacement without modifying the original
- `RelatedTo`: generic semantic association

**Implementation requirements:**
- **Both link directions must be created** for every relation: `CopyOf(A→B)` and `IsOriginalOf(B→A)`. Holochain's `get_links(base)` only finds outgoing links — relying on "find all links targeting X" at query time is expensive and unreliable.
- **SymlinkTo requires cycle detection** in the coordinator's resolution path (A → B → A produces infinite recursion). Either limit to one hop (non-recursive) or implement a visited-set.
- **Orphan handling**: when a file is deleted, all its relation links (both directions) must be deleted in the same `delete_file()` call, or the coordinator must handle dangling relations gracefully (return `None`, not an error).

---

## Cryptographic Proof of Integrity

Explicit, queryable guarantees over file and manifest integrity.

**What Holochain already provides (do not re-implement):**
- Every action is signed by the agent's private key
- Chunk hashes in `FileMetadata.chunks_hashes` are content addresses — if the chunk exists at that hash, the content is correct
- DHT validation enforces entry correctness network-wide

**Novel additions worth building:**
- **Multi-party attestation**: an `Attestation` entry type where an agent signs off that they have verified a file or manifest. Use case: "5 of 7 board members have verified this governance record." This is what Nondominium actually needs.
- **Manifest-level Merkle root**: compute a Merkle root over all file hashes in a `FileManifest`. Any agent can recompute and verify the manifest is complete and unmodified.
- **Cross-DNA verification**: expose a `verify_file_integrity(original_hash) → bool` function that other DNAs in the Technology Trinity can call without reading file chunks.

**Do not include:**
- External timestamp anchoring (anchoring hashes to Bitcoin or RFC 3161 services) — this requires calling external infrastructure and breaks Holochain's local-first model. If proof-of-existence-before-time-T is needed, it belongs in a bridge layer outside this zome, not in the zome itself.
