//! This module provides functions to handle file metadata and file chunks. It includes
//! operations such as creating, updating, and retrieving file metadata and chunks.
//! It also provides utility functions for handling file paths and chunking files.

use crate::get_file_metadata;
use file_system_integrity::*;
use hdk::prelude::*;
use regex::Regex;
use std::path::{Path as FileSystemPath, PathBuf};

/// Retrieves file metadata for all files within a given directory path and its subdirectories.
pub fn get_files_metadata_recursively(path: Path) -> ExternResult<Vec<Record>> {
    let mut files = Vec::new();

    let typed_path = path.clone().typed(LinkTypes::PathFileSystem)?;

    let files_links = get_links(
        LinkQuery::try_new(typed_path.path_entry_hash()?, LinkTypes::PathToFileMetaData)?,
        GetStrategy::default(),
    )?;

    for link in files_links {
        let file_metadata = get_file_metadata(link_target_action_hash(&link)?)?;
        if let Some(file_metadata) = file_metadata {
            files.push(file_metadata);
        }
    }

    let sub_folders_paths = typed_path.children_paths().unwrap_or_default();

    for path in sub_folders_paths {
        let sub_folder_files = get_files_metadata_recursively(path.path)?;
        files.extend(sub_folder_files)
    }

    Ok(files)
}

/// Creates a new file chunk and stores it in the DHT.
pub fn create_file_chunk(file_chunk: FileChunk) -> ExternResult<Record> {
    let file_chunk_hash = hash_entry(&file_chunk)?;

    if let None = get(file_chunk_hash.clone(), GetOptions::default())? {
        create_entry(&EntryTypes::FileChunk(file_chunk))?;
    }

    let record = get(file_chunk_hash.clone(), GetOptions::default())?.ok_or(wasm_error!(
        WasmErrorInner::Guest(String::from("Could not find the newly created file chunk"))
    ))?;

    Ok(record)
}

/// Creates a new file metadata entry and stores it in the DHT.
pub fn create_file_metadata(file_metadata: FileMetadata) -> ExternResult<Record> {
    let action_hash = create_entry(&EntryTypes::FileMetadata(file_metadata.clone()))?;
    let record =
        get_file_metadata(action_hash.clone())?.ok_or(wasm_error!(WasmErrorInner::Guest(
            String::from("Could not find the newly created file metadata")
        )))?;

    let file_path = fs_path_to_dht_path(file_metadata.path.as_str());
    let path = Path::from(file_path);
    let typed_path = path.typed(LinkTypes::PathFileSystem)?;
    typed_path.ensure()?;

    create_link(
        typed_path.path_entry_hash()?,
        action_hash.clone(),
        LinkTypes::PathToFileMetaData,
        (),
    )?;

    Ok(record)
}

/// Retrieves a file chunk by its hash from the DHT.
pub fn get_file_chunk(file_chunk_hash: EntryHash) -> ExternResult<Record> {
    let record = get(file_chunk_hash, GetOptions::default())?
        .ok_or(wasm_error!(WasmErrorInner::Guest("Chunk not found".into())))?;

    Ok(record)
}

/// Retrieves file metadata by path and name from the DHT.
pub fn get_file_metadata_by_path_and_name(path: String, name: String) -> ExternResult<Record> {
    let file_path = fs_path_to_dht_path(path.as_str());
    let path = Path::from(file_path);
    let typed_path = path.typed(LinkTypes::PathFileSystem)?;
    let files_links = get_links(
        LinkQuery::try_new(typed_path.path_entry_hash()?, LinkTypes::PathToFileMetaData)?,
        GetStrategy::default(),
    )?;

    for link in files_links {
        let file_metadata_record = get_file_metadata(link_target_action_hash(&link)?)?
            .ok_or(wasm_error!(WasmErrorInner::Guest(String::from(
                "Could not find the newly created file metadata"
            ))))?;
        let file_metadata: FileMetadata = file_metadata_record.clone().try_into()?;
        if file_metadata.name == name {
            return Ok(file_metadata_record);
        }
    }

    Err(wasm_error!(WasmErrorInner::Guest("File not found".into())))
}

/// Updates the file metadata for a given file.
pub fn update_file_metadata(
    original_file_metadata_hash: ActionHash,
    previous_file_metadata_hash: Option<ActionHash>,
    file_metadata: FileMetadata,
) -> ExternResult<Record> {
    let file_metadata_hash =
        previous_file_metadata_hash.unwrap_or_else(|| original_file_metadata_hash.clone());
    let updated_metadata_hash = update_entry(file_metadata_hash, &file_metadata.clone())?;

    create_link(
        original_file_metadata_hash,
        updated_metadata_hash.clone(),
        LinkTypes::FileMetaDataUpdate,
        (),
    )?;

    let record = get_file_metadata(updated_metadata_hash.clone())?.ok_or(wasm_error!(
        WasmErrorInner::Guest(String::from("Could not find the file metadata"))
    ))?;

    Ok(record)
}

/// Returns the target of a link as an `ActionHash`, failing if the link points at another hash type.
pub fn link_target_action_hash(link: &Link) -> ExternResult<ActionHash> {
    link.target
        .clone()
        .into_action_hash()
        .ok_or(wasm_error!(WasmErrorInner::Guest(String::from(
            "Link target is not an action hash"
        ))))
}

/// Deletes the `PathToFileMetaData` link that indexes a file under its directory path.
pub fn delete_path_link(fs_path: &str, original_file_metadata_hash: &ActionHash) -> ExternResult<Vec<ActionHash>> {
    let typed_path = Path::from(fs_path_to_dht_path(fs_path)).typed(LinkTypes::PathFileSystem)?;
    let links = get_links(
        LinkQuery::try_new(typed_path.path_entry_hash()?, LinkTypes::PathToFileMetaData)?,
        GetStrategy::default(),
    )?;

    let mut deleted = Vec::new();
    for link in links {
        if link.target.clone().into_action_hash().as_ref() == Some(original_file_metadata_hash) {
            deleted.push(delete_link(link.create_link_hash, GetOptions::default())?);
        }
    }
    Ok(deleted)
}

/// Splits the file content into chunks and returns a vector of their hashes.
pub fn chunk_file(file_content: Vec<u8>) -> ExternResult<Vec<EntryHash>> {
    let chunk_size = 1024 * 1024; // 1 MB
    let num_chunks = (file_content.len() as f64 / chunk_size as f64).ceil() as usize;
    let mut chunks_hashes = Vec::new();

    for i in 0..num_chunks {
        let start = i * chunk_size;
        let end = std::cmp::min((i + 1) * chunk_size, file_content.len());
        let chunk_data = file_content[start..end].to_vec();

        let file_chunk = FileChunk(SerializedBytes::from(UnsafeBytes::from(chunk_data)));

        create_file_chunk(file_chunk.clone())?;
        let chunk_hash = hash_entry(&file_chunk)?;
        chunks_hashes.push(chunk_hash);
    }

    Ok(chunks_hashes)
}

/// Converts a filesystem-style path to a DHT-style path.
pub fn fs_path_to_dht_path(path: &str) -> String {
    let mut path = standardize_fs_path(path);
    path.remove(0);

    let mut path_parts = path.split("/").collect::<Vec<&str>>();
    if path_parts[0] == "" {
        path_parts.remove(0);
    }
    path_parts.insert(0, "root");
    path_parts.join(".")
}

/// Returns a standardized filesystem path by converting backslashes to forward slashes and removing redundant separators.
pub fn standardize_fs_path(path: &str) -> String {
    let path = path.replace("\\", "/");
    let re = Regex::new(r"[/]+").unwrap();
    let path = re.replace_all(&path, "/").to_string();
    let mut path = PathBuf::from(path);

    let main_separator = std::path::MAIN_SEPARATOR.to_string();

    if !path.has_root() {
        path = FileSystemPath::new(&main_separator).join(path);
    }

    let path_str = path.to_str().unwrap();
    let trimmed_path = path_str.trim_end_matches(&main_separator);
    if trimmed_path == "" {
        return main_separator;
    }

    trimmed_path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fs_path_to_dht_path() {
        let cases = vec![
            ("/test/path", "root.test.path"),
            ("test/path", "root.test.path"),
            ("/test//path", "root.test.path"),
            ("/test/path/", "root.test.path"),
            ("", "root"),
            ("/", "root"),
            ("///", "root"),
        ];

        for (input, expected) in cases {
            let dht_path = fs_path_to_dht_path(input);
            assert_eq!(dht_path, expected, "Failed for input: {:?}", input);
        }
    }

    #[test]
    fn test_standardize_fs_path() {
        let cases = vec![
            ("/test/path/", "/test/path"),
            ("/test///path//", "/test/path"),
            ("test\\path\\", "/test/path"),
            ("test\\\\path\\\\", "/test/path"),
            ("\\test\\path", "/test/path"),
            ("test/path", "/test/path"),
            ("/", "/"),
            ("", "/"),
            ("\\", "/"),
            ("//", "/"),
            ("\\\\", "/"),
        ];

        for (input, expected) in cases {
            let standardized_fs_path = standardize_fs_path(input);
            assert_eq!(
                standardized_fs_path, expected,
                "Failed for input: {:?}",
                input
            );
        }
    }
}
