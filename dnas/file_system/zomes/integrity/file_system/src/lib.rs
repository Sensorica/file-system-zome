//! Integrity zome of the file system: the entry types, link types and validation rules
//! every peer of the DNA agrees on.
//!
//! A file is stored as one [`FileMetadata`] entry plus the [`FileChunk`] entries holding its
//! content in pieces of up to 1 MB. Files are indexed under a tree of Holochain paths
//! ([`LinkTypes::PathFileSystem`], [`LinkTypes::PathToFileMetaData`]) and versioned through
//! update links ([`LinkTypes::FileMetaDataUpdate`]). The coordinator zome `file_system` owns
//! the logic that writes them; this crate only defines their shape and what makes them valid.
//!
//! # Validation
//!
//! [`validate`] runs on every peer that stores a new or updated [`FileMetadata`] and rejects it
//! when:
//!
//! - its `name` is empty (`"File name cannot be empty"`);
//! - its `path` contains one of `< > : " | ? * .` (`"File path cannot contain forbidden characters"`).
//!
//! Chunks and links are accepted as they are. Ownership (who may update or delete a file) and
//! the immutability of `name`, `path` and `file_type` across versions are not enforced yet.

// Every item written here is documented. `missing_docs` is not enabled because the
// `hdk_entry_types`, `hdk_link_types` and `UnitEnum` macros generate public items that cannot
// carry docs, and the lint would only report those.

use hdi::prelude::*;

/// The links this zome creates.
#[derive(Serialize, Deserialize)]
#[hdk_link_types]
pub enum LinkTypes {
    /// Parent-to-child links of the directory tree, created by `TypedPath::ensure` when a file is
    /// stored under a path. `/docs/reports` is the DHT path `root.docs.reports`.
    PathFileSystem,
    /// From a directory's path entry to the original create action of each [`FileMetadata`]
    /// stored directly in that directory. Removed when the file is deleted.
    PathToFileMetaData,
    /// From a file's original create action to each later version of its [`FileMetadata`].
    /// The newest link by timestamp is the current version.
    FileMetaDataUpdate,
}

/// The entries this zome stores, both public.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
#[hdk_entry_types]
#[unit_enum(UnitEntryTypes)]
pub enum EntryTypes {
    /// Description of one version of a file.
    FileMetadata(FileMetadata),
    /// A piece of a file's content.
    FileChunk(FileChunk),
}

/// Up to 1 MB of a file's raw content.
///
/// Chunks are content-addressed: two chunks with the same bytes have the same entry hash, and the
/// coordinator stores each distinct chunk only once. A [`FileMetadata`] lists its chunks, in order,
/// in [`FileMetadata::chunks_hashes`].
#[hdk_entry_helper]
#[derive(Clone)]
pub struct FileChunk(
    /// The raw bytes, kept as-is (a msgpack binary on the wire).
    pub SerializedBytes,
);

/// One version of a file: where it lives, what it is, and which chunks hold its content.
///
/// The first version is created with `create_entry`; each later version is an `update_entry` of
/// the previous one plus a [`LinkTypes::FileMetaDataUpdate`] link from the original. The original
/// create action's hash therefore identifies the file across all its versions.
#[hdk_entry_helper]
#[derive(Clone)]
pub struct FileMetadata {
    /// File name, without its directory. Must not be empty. May contain dots.
    pub name: String,
    /// Agent who created the first version. Duplicates the action author; planned to move out of
    /// the entry.
    pub author: AgentPubKey,
    /// Standardized directory path, e.g. `/docs/reports`, `/` for the root. Must not contain
    /// `< > : " | ? * .`.
    pub path: String,
    /// When the first version was created. Duplicates the create action's timestamp.
    pub created: Timestamp,
    /// When this version was written. Duplicates the update action's timestamp.
    pub last_modified: Timestamp,
    /// Total content size in bytes for this version.
    pub size: usize,
    /// MIME type or extension, as given by the caller. Not interpreted.
    pub file_type: String,
    /// Entry hashes of the [`FileChunk`]s holding this version's content, in order. Empty for an
    /// empty file.
    pub chunks_hashes: Vec<EntryHash>,
}

/// Validation callback, run by every peer that receives an operation of this zome.
///
/// Only the store-entry operation of a created or updated [`FileMetadata`] is checked, against the
/// rules listed in the [crate documentation](crate#validation). Every other operation is valid.
///
/// # Errors
///
/// Returns an error only when the operation cannot be deserialized into this zome's types; a rule
/// violation is `Ok(ValidateCallbackResult::Invalid(reason))`.
#[hdk_extern]
pub fn validate(op: Op) -> ExternResult<ValidateCallbackResult> {
    match op.flattened::<EntryTypes, LinkTypes>()? {
        FlatOp::CreateEntry(OpEntry::CreateEntry { app_entry, .. })
        | FlatOp::CreateEntry(OpEntry::UpdateEntry { app_entry, .. }) => match app_entry {
            EntryTypes::FileMetadata(file_metadata) => {
                validate_create_file_metadata(file_metadata)
            }
            EntryTypes::FileChunk(_) => Ok(ValidateCallbackResult::Valid),
        },
        _ => Ok(ValidateCallbackResult::Valid),
    }
}

/// Applies the [`FileMetadata`] rules: non-empty name, no forbidden character in the path.
fn validate_create_file_metadata(
    file_metadata: FileMetadata,
) -> ExternResult<ValidateCallbackResult> {
    if file_metadata.name.is_empty() {
        return Ok(ValidateCallbackResult::Invalid(
            "File name cannot be empty".into(),
        ));
    }

    if has_forbidden_chars(file_metadata.path.as_str()) {
        return Ok(ValidateCallbackResult::Invalid(
            "File path cannot contain forbidden characters".into(),
        ));
    }

    Ok(ValidateCallbackResult::Valid)
}

/// Returns `true` when `path` contains one of `< > : " | ? * .`.
fn has_forbidden_chars(path: &str) -> bool {
    let forbidden_chars = &['<', '>', ':', '"', '|', '?', '*', '.'];
    path.chars().any(|c| forbidden_chars.contains(&c))
}
