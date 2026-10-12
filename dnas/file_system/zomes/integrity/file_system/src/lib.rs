//! This module provides a file storage integrity zome for the Holochain ecosystem.
//! It includes entry types, link types, and validation logic for file metadata and chunks.

use hdi::prelude::*;

/// Defines link types for the integrity zome.
#[derive(Serialize, Deserialize)]
#[hdk_link_types]
pub enum LinkTypes {
    PathFileSystem,
    PathToFileMetaData,
    FileMetaDataUpdate,
}

/// Defines entry types for the integrity zome.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
#[hdk_entry_types]
#[unit_enum(UnitEntryTypes)]
pub enum EntryTypes {
    FileMetadata(FileMetadata),
    FileChunk(FileChunk),
}

/// File chunk entry type.
#[hdk_entry_helper]
#[derive(Clone)]
pub struct FileChunk(pub SerializedBytes);

/// File metadata entry type.
#[hdk_entry_helper]
#[derive(Clone)]
pub struct FileMetadata {
    pub name: String,
    pub author: AgentPubKey,
    pub path: String,
    pub created: Timestamp,
    pub last_modified: Timestamp,
    pub size: usize,
    pub file_type: String,
    pub chunks_hashes: Vec<EntryHash>,
}

/// Validates the provided `Op` to ensure the entry and link types adhere to the defined constraints.
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

/// Validates the `FileMetadata` for its creation or update, ensuring the name and path meet the defined constraints.
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

/// Checks if a given path contains any forbidden characters, returning `true` if any are found.
fn has_forbidden_chars(path: &str) -> bool {
    let forbidden_chars = &['<', '>', ':', '"', '|', '?', '*', '.'];
    path.chars().any(|c| forbidden_chars.contains(&c))
}
