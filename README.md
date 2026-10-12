# File System Zome

[![test](https://github.com/Sensorica/file-system-zome/actions/workflows/test.yaml/badge.svg)](https://github.com/Sensorica/file-system-zome/actions/workflows/test.yaml)
![Holochain 0.7](https://img.shields.io/badge/holochain-0.7.0-blue)

A Holochain building block for organizing files: it stores files in the DHT, indexes them under hierarchical paths, keeps a version chain for every file, and emits signals when files change. It is meant to be embedded in other DNAs so apps do not each rebuild their own file organization.

## Status

**Proof of concept, running on Holochain 0.7** (`hdk 0.7.0`, `hdi 0.8.0`, holonix `main-0.7`). The data model and algorithms are working and tested with two agents; the API is not frozen yet and the crates are not published.

| Capability | Covered by (Sweettest, two agents) |
|---|---|
| Create a file under a path; Bob reads what Alice wrote | `create_files_and_list_them_by_path` |
| Recursive listing: a directory returns its files and those of every subdirectory | `create_files_and_list_them_by_path` |
| Content split into 1 MB chunks and reassembled byte for byte | `large_file_is_chunked_and_deleted` |
| Version chain: updates link to the original, reads resolve to the newest version | `update_file_twice_then_delete_in_cascade` |
| Updates keep the chunks the new version reuses (re-save, append) | `update_keeps_chunks_the_new_version_reuses` |
| Update and delete only accept a file's original hash | `update_and_delete_require_the_original_hash` |
| Duplicate path and name refused | `update_file_twice_then_delete_in_cascade` |
| Delete removes every version, its chunks and its directory entry | `update_file_twice_then_delete_in_cascade`, `deleted_file_leaves_its_directory_listing` |
| Validation: empty names and forbidden path characters rejected at authoring and by entry authorities | `validation_and_path_standardization` |
| Path standardization: `\`, `/` and repeated separators normalized | `validation_and_path_standardization`, unit tests in `files.rs` |
| Signals on create, update and delete of file metadata | `file_metadata_changes_emit_signals` |

### Known limitations

These are real and tracked; they are the next iteration's work, not hidden behaviour.

- **Shared chunks.** Chunks are deduplicated by content, so two files with an identical 1 MB chunk share one chunk record. Deleting or updating one of them deletes the chunk the other still points to.
- **No ownership rules.** Validation does not check that only the author updates or deletes a file, and does not enforce that `name`, `path` and `file_type` stay unchanged across versions.
- **Concurrent creates.** Two agents creating the same path and name at the same moment both succeed; the duplicate check only sees what has already synced.
- **Recursive listing walks the whole subtree** on every call; there is no non-recursive listing yet.
- **`FileMetadata` still carries `author`, `created` and `last_modified`**, which duplicate the action header. The target model moves them out (see Roadmap).

## Quick start

Requires [Nix](https://nixos.org/download/) with flakes enabled. Everything else (Holochain 0.7, Rust with the wasm target, Node 24) comes from the dev shell.

```bash
nix develop
npm install
npm test     # builds the zomes, packs the hApp, runs the unit and Sweettest suites
npm start    # local bootstrap/relay server plus two conductors running the hApp
```

`AGENTS=3 npm start` runs three conductors instead of two. Each conductor prints its admin port and the app port a `@holochain/client` connects to.

To call the zome by hand while `npm start` runs, from a second `nix develop` shell (the dev passphrase is `pass`; `ADMIN_PORT_0`, `ADMIN_PORT_1` are the admin ports printed on the `Conductor launched` lines, `DNA_HASH` is the `uhC0k…` hash shown by `list-apps`):

```bash
hc client call --port ADMIN_PORT_0 list-apps
echo pass | hc client zome-call-auth --piped --port ADMIN_PORT_0 test-app
echo pass | hc client zome-call --piped --port ADMIN_PORT_0 test-app DNA_HASH file_system create_file '{"name":"hello.txt","path":"/demo","file_type":"text/plain","content":[104,105]}'
echo pass | hc client zome-call-auth --piped --port ADMIN_PORT_1 test-app
echo pass | hc client zome-call --piped --port ADMIN_PORT_1 test-app DNA_HASH file_system get_files_metadata_by_path_recursively '"/demo"'
```

The last call runs on the second conductor and returns the file the first one created, once the two peers have synced.

Other scripts:

```bash
npm run build:zomes   # wasm only
npm run build:happ    # wasm + workdir/soushi-cloud.happ
cargo test -p file_system --test file_system create_files_and_list_them_by_path   # one scenario
```

## Using it in your DNA

Until the crates are published, add both zomes to your DNA from this repository:

1. Build the wasm with `npm run build:zomes`.
2. Reference `file_system_integrity.wasm` under `integrity.zomes` and `file_system.wasm` under `coordinator.zomes` in your `dna.yaml`, with the coordinator depending on `file_system_integrity`. [`dnas/file_system/workdir/dna.yaml`](dnas/file_system/workdir/dna.yaml) is a working example.
3. Call the functions below with zome name `file_system`.

## Data model

### Entries

**`FileMetadata`**

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | File name |
| `author` | `AgentPubKey` | Original creator's public key |
| `path` | `String` | Directory path, standardized (e.g. `/documents/reports`) |
| `created` | `Timestamp` | Creation time |
| `last_modified` | `Timestamp` | Last update time |
| `size` | `usize` | Total file size in bytes |
| `file_type` | `String` | MIME type or extension |
| `chunks_hashes` | `Vec<EntryHash>` | Ordered chunk entry hashes |

**`FileChunk`**: raw bytes, up to 1 MB.

### Links

| Link | Base | Target | Purpose |
|------|------|--------|---------|
| `PathFileSystem` | Path entry | Path entry | DHT path tree for the directory hierarchy |
| `PathToFileMetaData` | Path entry | `FileMetadata` action hash | Directory to file index |
| `FileMetaDataUpdate` | Original metadata hash | Updated metadata hash | Version chain |

### Paths

Filesystem paths map onto Holochain's `Path` tree:

```
/documents/reports  →  root.documents.reports
```

Separator `/` in filesystem paths, `.` in DHT paths; every path hangs off `root`. Forbidden characters in paths: `< > : " | ? * .` (file names may contain dots).

### Versions

Updates never mutate an entry. Each update writes a new `FileMetadata` with `update_entry`, links it from the original with `FileMetaDataUpdate`, and deletes the previous version's chunks:

```
[original] ──FileMetaDataUpdate──> [v2]
           ──FileMetaDataUpdate──> [v3]
```

`get_file_metadata(original_hash)` follows the newest link, so the original hash is a stable file identifier.

### Signals

Emitted from `post_commit` for `FileMetadata` actions only: `FileMetadataCreated`, `FileMetadataUpdated`, `FileMetadataDeleted`.

## Public API

| Function | Input | Output |
|---|---|---|
| `create_file` | `FileInput { name, path, file_type, content }` | `FileOutput { file_metadata, file_chunks }` |
| `update_file` | `UpdateFileMetadataInput { original_file_metadata_hash, new_content }` | `FileOutput` |
| `delete_file` | original `ActionHash` | `Vec<ActionHash>` of the delete actions |
| `get_file_metadata` | original `ActionHash` | `Option<Record>`, latest version |
| `get_file_chunks` | original `ActionHash` | `Vec<Record>`, chunks of the latest version |
| `get_files_metadata_by_path_recursively` | path `String` | `Vec<Record>` |

`create_file` fails if a file with the same name already exists at that path. `content` is `SerializedBytes` (a msgpack byte array from a JavaScript client).

## Roadmap

The target is a **two-layer architecture**: chunk storage delegated to the community [`holochain-open-dev/file-storage`](https://github.com/holochain-open-dev/file-storage) zome, with this project as the file system layer on top (paths, authorship, versions, signals), so several apps can share one storage provider while keeping their own namespaces. None of that delegation exists yet.

- [`documentation/ARCHITECTURE.md`](documentation/ARCHITECTURE.md): the target design.
- [`documentation/SPECIFICATION.md`](documentation/SPECIFICATION.md): the phased plan. Phase 1 (HDK migration) and Phase 3 (bug fixes) are done.
- [`documentation/REQUIREMENTS.md`](documentation/REQUIREMENTS.md): requirements and integration context.
- [`documentation/IDEAS.md`](documentation/IDEAS.md): later capabilities.
- [`CHANGELOG.md`](CHANGELOG.md): what changed, release by release.

## Attribution

Inspired by the [file-storage zome](https://github.com/holochain-open-dev/file-storage) from [Holochain Open Dev](https://holochain-open-dev.github.io/). Maintained by [Sensorica](https://www.sensorica.co/).
