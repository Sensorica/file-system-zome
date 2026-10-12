//! Coordinator zome of the file system: the functions an app calls to store, version, list and
//! delete files in the DHT.
//!
//! Entry and link types come from the `file_system_integrity` crate. Zome name: `file_system`.
//!
//! # Model
//!
//! - A file is identified by the [`ActionHash`] of the action that created its first
//!   [`FileMetadata`] version, called the *original hash* below. Every function taking a file
//!   takes that hash; [`create_file`] returns it as `file_metadata.signed_action.hashed.hash`.
//! - Content is split into 1 MB [`FileChunk`]s, deduplicated by content.
//! - A file lives in a directory given as a filesystem path (`/docs/reports`). Paths are
//!   standardized (`\` becomes `/`, repeated and trailing separators collapse) and mapped to the
//!   DHT path `root.docs.reports`; see [`files::fs_path_to_dht_path`].
//! - [`update_file`] writes a new version and links it from the original;
//!   [`get_file_metadata`] and [`get_file_chunks`] always resolve the newest one.
//! - [`post_commit`](signals::post_commit) emits a [`signals::Signal`] for every created, updated
//!   or deleted [`FileMetadata`].
//!
//! # Functions
//!
//! | Function | Input | Output |
//! |---|---|---|
//! | [`create_file`] | [`FileInput`] | [`FileOutput`] |
//! | [`update_file`] | [`UpdateFileMetadataInput`] | [`FileOutput`] |
//! | [`delete_file`] | original hash | delete action hashes |
//! | [`get_file_metadata`] | original hash | latest version, or `None` |
//! | [`get_file_chunks`] | original hash | chunks of the latest version |
//! | [`get_files_metadata_by_path_recursively`] | directory path | every file below it |
//!
//! # Errors
//!
//! Failures reach the caller as `WasmErrorInner::Guest` with one of these messages:
//!
//! | Message | Raised by |
//! |---|---|
//! | `File already exists` | [`create_file`]: same name already in that directory |
//! | `File not found` | [`update_file`], [`delete_file`]: unknown or deleted original hash |
//! | `Expected the original file metadata hash, got a later version` | [`update_file`], [`delete_file`] |
//! | `Chunk not found` | any function reading a chunk that is not reachable |
//!
//! Validation failures from the integrity zome (`File name cannot be empty`, `File path cannot
//! contain forbidden characters`) surface on [`create_file`] and [`update_file`].

#![warn(missing_docs)]

use file_system_integrity::*;
use files::*;
use hdk::prelude::*;

pub mod files;
pub mod signals;

/// Input of [`create_file`].
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct FileInput {
    /// File name, without its directory. Must not be empty.
    pub name: String,
    /// Directory to store the file in, e.g. `/docs`. Standardized before use, so `docs`,
    /// `\docs\` and `//docs/` all mean `/docs`.
    pub path: String,
    /// MIME type or extension, stored as given.
    pub file_type: String,
    /// The file's content. From a JavaScript client, a `Uint8Array`.
    pub content: SerializedBytes,
}

/// Output of [`create_file`] and [`update_file`].
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct FileOutput {
    /// The [`FileMetadata`] record just written. For [`create_file`], its action hash is the
    /// file's original hash.
    pub file_metadata: Record,
    /// The [`FileChunk`] records of this version, in content order. Repeated content yields the
    /// same record more than once.
    pub file_chunks: Vec<Record>,
}

/// Stores a new file under `path`, chunking its content.
///
/// Writes the chunks, a [`FileMetadata`] entry, the directory path entries and a
/// [`LinkTypes::PathToFileMetaData`] link from the directory to the file.
///
/// # Errors
///
/// - `File already exists` when a live file with the same name is already visible in that
///   directory. Two agents creating the same name concurrently can both succeed.
/// - The integrity zome's validation messages for an empty name or a forbidden path character.
#[hdk_extern]
pub fn create_file(file_input: FileInput) -> ExternResult<FileOutput> {
    let already_created_record =
        get_file_metadata_by_path_and_name(file_input.path.clone(), file_input.name.clone());

    if already_created_record.is_ok() {
        return Err(wasm_error!(WasmErrorInner::Guest(String::from(
            "File already exists"
        ))));
    }

    let chunks_hashes = chunk_file(file_input.content.bytes().clone())?;

    let file_metadata = FileMetadata {
        name: file_input.name.clone(),
        author: agent_info()?.agent_initial_pubkey,
        path: standardize_fs_path(&file_input.path),
        created: sys_time()?,
        last_modified: sys_time()?,
        size: file_input.content.bytes().len(),
        file_type: file_input.file_type.clone(),
        chunks_hashes: chunks_hashes.clone(),
    };

    let metadata_record = create_file_metadata(file_metadata)?;
    let chunks_records = chunks_hashes
        .iter()
        .map(|chunk_hash| get_file_chunk(chunk_hash.clone()))
        .collect::<ExternResult<Vec<Record>>>()?;
    let records = FileOutput {
        file_metadata: metadata_record,
        file_chunks: chunks_records,
    };

    Ok(records)
}

/// Returns the chunks of the newest version of a file, in content order.
///
/// Concatenating each chunk's bytes gives the file's content. Returns an empty list when the file
/// does not exist, was deleted, or is empty.
///
/// # Errors
///
/// `Chunk not found` when the metadata lists a chunk that cannot be fetched.
#[hdk_extern]
pub fn get_file_chunks(file_metadata_hash: ActionHash) -> ExternResult<Vec<Record>> {
    let Some(record) = get_file_metadata(file_metadata_hash)? else {
        return Ok(Vec::new());
    };

    let file_metadata: FileMetadata = record.try_into()?;

    let mut file_chunks = Vec::new();

    if file_metadata.chunks_hashes.is_empty() {
        return Ok(file_chunks);
    }

    for file_chunk_hash in file_metadata.chunks_hashes {
        let file_chunk = get_file_chunk(file_chunk_hash)?;
        file_chunks.push(file_chunk);
    }

    Ok(file_chunks)
}

/// Returns the newest version of a file's [`FileMetadata`] record, or `None` once it is deleted.
///
/// Follows the [`LinkTypes::FileMetaDataUpdate`] link with the latest timestamp from the original
/// hash; with no update, the original record is the newest.
///
/// # Errors
///
/// Network or deserialization errors from the host only.
#[hdk_extern]
pub fn get_file_metadata(original_file_metadata_hash: ActionHash) -> ExternResult<Option<Record>> {
    let latest_file_metadata_hash = latest_file_metadata_hash(&original_file_metadata_hash)?
        .unwrap_or(original_file_metadata_hash);

    get(latest_file_metadata_hash, GetOptions::default())
}

/// Lists the newest [`FileMetadata`] record of every file in a directory and all of its
/// subdirectories.
///
/// `path_string` is a filesystem path, standardized like [`FileInput::path`]; `/` lists every
/// file. Deleted files are not listed. The whole subtree is walked on every call.
///
/// # Errors
///
/// Network or deserialization errors from the host only.
#[hdk_extern]
pub fn get_files_metadata_by_path_recursively(path_string: String) -> ExternResult<Vec<Record>> {
    let path_string = fs_path_to_dht_path(path_string.as_str());
    let path = Path::from(path_string);

    get_files_metadata_recursively(path)
}

/// Input of [`update_file`].
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct UpdateFileMetadataInput {
    /// The file's original hash, never a later version's.
    pub original_file_metadata_hash: ActionHash,
    /// The complete new content, replacing the previous one.
    pub new_content: SerializedBytes,
}

/// Replaces a file's content with a new version.
///
/// Chunks the new content, writes a new [`FileMetadata`] version as an update of the newest one,
/// links it from the original with [`LinkTypes::FileMetaDataUpdate`], and deletes the previous
/// version's chunks except those the new version reuses. `name`, `path`, `file_type`, `author`
/// and `created` carry over; `size` and `last_modified` are recomputed.
///
/// # Errors
///
/// - `File not found` for an unknown or deleted hash.
/// - `Expected the original file metadata hash, got a later version` when given a version hash.
/// - `Chunk not found` when a previous chunk cannot be fetched.
#[hdk_extern]
pub fn update_file(
    update_file_metadata_input: UpdateFileMetadataInput,
) -> ExternResult<FileOutput> {
    let original_file_metadata_hash = update_file_metadata_input.original_file_metadata_hash;
    let new_content = update_file_metadata_input.new_content.bytes();
    ensure_original_version(&original_file_metadata_hash)?;

    let previous_file_metadata_hash = latest_file_metadata_hash(&original_file_metadata_hash)?;

    let chunks_hashes = chunk_file(new_content.to_vec())?;
    let now = sys_time()?;

    let file_metadata_record =
        get_file_metadata(original_file_metadata_hash.clone())?.ok_or(wasm_error!(
            WasmErrorInner::Guest(String::from("Could not find the file metadata"))
        ))?;
    let mut file_metadata = FileMetadata::try_from(file_metadata_record.clone())?;
    let old_chunks_hashes = file_metadata.chunks_hashes.clone();

    file_metadata.last_modified = now;
    file_metadata.size = new_content.len();
    file_metadata.chunks_hashes = chunks_hashes.clone();

    // Chunks are content-addressed: keep the ones the new version reuses, and delete each other one once.
    let mut deleted_chunks = Vec::new();
    for chunk_hash in old_chunks_hashes {
        if chunks_hashes.contains(&chunk_hash) || deleted_chunks.contains(&chunk_hash) {
            continue;
        }
        let chunk_record = get_file_chunk(chunk_hash.clone())?;
        delete_entry(chunk_record.signed_action.hashed.hash)?;
        deleted_chunks.push(chunk_hash);
    }

    let updated_metadata_record = update_file_metadata(
        original_file_metadata_hash.clone(),
        previous_file_metadata_hash.clone(),
        file_metadata,
    )?;

    let chunks_records = chunks_hashes
        .iter()
        .map(|chunk_hash| get_file_chunk(chunk_hash.clone()))
        .collect::<ExternResult<Vec<Record>>>()?;
    let records = FileOutput {
        file_metadata: updated_metadata_record,
        file_chunks: chunks_records,
    };

    Ok(records)
}

/// Deletes a file: every version, the chunks of the newest version, and its directory link.
///
/// Older versions' chunks were already deleted when they were superseded. After this call the
/// file leaves [`get_files_metadata_by_path_recursively`], [`get_file_metadata`] returns `None`,
/// and its name is free again in that directory.
///
/// Returns the hashes of every delete and delete-link action written.
///
/// # Errors
///
/// - `File not found` for an unknown or already deleted hash.
/// - `Expected the original file metadata hash, got a later version` when given a version hash.
#[hdk_extern]
pub fn delete_file(original_file_metadata_hash: ActionHash) -> ExternResult<Vec<ActionHash>> {
    let mut delete_actions: Vec<ActionHash> = Vec::new();

    // `path` never changes across versions, so the original entry tells us where the file is indexed.
    let original_record = ensure_original_version(&original_file_metadata_hash)?;
    let original_file_metadata = FileMetadata::try_from(original_record)?;
    delete_actions.extend(delete_path_link(
        &original_file_metadata.path,
        &original_file_metadata_hash,
    )?);

    let update_links = get_links(
        LinkQuery::try_new(
            original_file_metadata_hash.clone(),
            LinkTypes::FileMetaDataUpdate,
        )?,
        GetStrategy::default(),
    )?;
    let latest_metadata_hash = latest_file_metadata_hash(&original_file_metadata_hash)?;

    if let Some(latest_metadata_hash) = latest_metadata_hash {
        for link in update_links {
            let version_hash = link_target_action_hash(&link)?;
            if version_hash != latest_metadata_hash {
                delete_actions.push(delete_entry(version_hash)?);
            }
        }

        delete_actions.extend(delete_chunks_of(latest_metadata_hash.clone())?);
        delete_actions.push(delete_entry(latest_metadata_hash)?);
    } else {
        delete_actions.extend(delete_chunks_of(original_file_metadata_hash.clone())?);
    }

    delete_actions.push(delete_entry(original_file_metadata_hash)?);

    Ok(delete_actions)
}

/// Deletes the chunks of one file version, each chunk record once even when the file repeats a chunk.
fn delete_chunks_of(file_metadata_hash: ActionHash) -> ExternResult<Vec<ActionHash>> {
    let mut deleted: Vec<ActionHash> = Vec::new();
    for file_chunk in get_file_chunks(file_metadata_hash)? {
        let chunk_action_hash = file_chunk.signed_action.hashed.hash;
        if !deleted.contains(&chunk_action_hash) {
            delete_entry(chunk_action_hash.clone())?;
            deleted.push(chunk_action_hash);
        }
    }
    Ok(deleted)
}

/// Returns the record behind `hash`, failing unless it is the create action that identifies a file.
/// Update and delete take the original hash; a later version's hash would leave the path index dangling.
fn ensure_original_version(hash: &ActionHash) -> ExternResult<Record> {
    let record = get(hash.clone(), GetOptions::default())?
        .ok_or(wasm_error!(WasmErrorInner::Guest(String::from("File not found"))))?;
    match &record.action().data {
        ActionData::Create(_) => Ok(record),
        _ => Err(wasm_error!(WasmErrorInner::Guest(String::from(
            "Expected the original file metadata hash, got a later version"
        )))),
    }
}

/// Returns the hash of the most recent version of a file, or `None` if it was never updated.
fn latest_file_metadata_hash(
    original_file_metadata_hash: &ActionHash,
) -> ExternResult<Option<ActionHash>> {
    let links = get_links(
        LinkQuery::try_new(
            original_file_metadata_hash.clone(),
            LinkTypes::FileMetaDataUpdate,
        )?,
        GetStrategy::default(),
    )?;

    links
        .into_iter()
        .max_by(|link_a, link_b| link_a.timestamp.cmp(&link_b.timestamp))
        .map(|link| link_target_action_hash(&link))
        .transpose()
}
