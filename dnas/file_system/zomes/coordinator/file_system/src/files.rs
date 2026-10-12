//! Building blocks behind the zome functions: chunk storage, metadata writes, directory
//! traversal, and the conversion between filesystem paths and DHT paths.
//!
//! These are plain Rust functions, not zome externs. They are public so another coordinator
//! linking this crate can reuse them, and they assume the caller already checked its inputs.

use crate::get_file_metadata;
use file_system_integrity::*;
use hdk::prelude::*;
use regex::Regex;
use std::path::{Path as FileSystemPath, PathBuf};

/// Returns the newest [`FileMetadata`] record of every live file under `path` (a DHT path, see
/// [`fs_path_to_dht_path`]), descending into every child path.
///
/// Files whose metadata is deleted or not yet visible are skipped.
///
/// # Errors
///
/// Host errors from `get_links` or `get`, or a link target that is not an action hash.
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

/// Stores `file_chunk` unless an identical chunk is already live, and returns its record.
///
/// Because chunks are content-addressed, an existing record (possibly written for another file)
/// is returned as is. That sharing is why deleting one file can remove a chunk another file with
/// identical content still lists.
///
/// # Errors
///
/// Host errors, or `Could not find the newly created file chunk` if the chunk is not readable
/// right after being written.
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

/// Writes the first version of a file: the [`FileMetadata`] entry, its directory path entries, and
/// the [`LinkTypes::PathToFileMetaData`] link from the directory. Returns the metadata record.
///
/// Does not check for an existing file of the same name; [`create_file`](crate::create_file) does.
///
/// # Errors
///
/// Host or validation errors from the writes.
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

/// Fetches a live [`FileChunk`] record by entry hash.
///
/// # Errors
///
/// `Chunk not found` when no live record holds that entry.
pub fn get_file_chunk(file_chunk_hash: EntryHash) -> ExternResult<Record> {
    let record = get(file_chunk_hash, GetOptions::default())?
        .ok_or(wasm_error!(WasmErrorInner::Guest("Chunk not found".into())))?;

    Ok(record)
}

/// Finds the newest [`FileMetadata`] record of the live file called `name` directly in the
/// directory `path` (a filesystem path, standardized here).
///
/// # Errors
///
/// `File not found` when no visible live file has that name; host errors otherwise.
pub fn get_file_metadata_by_path_and_name(path: String, name: String) -> ExternResult<Record> {
    let file_path = fs_path_to_dht_path(path.as_str());
    let path = Path::from(file_path);
    let typed_path = path.typed(LinkTypes::PathFileSystem)?;
    let files_links = get_links(
        LinkQuery::try_new(typed_path.path_entry_hash()?, LinkTypes::PathToFileMetaData)?,
        GetStrategy::default(),
    )?;

    for link in files_links {
        // A link whose file is gone (deleted, or not yet visible) does not hold the name.
        let Some(file_metadata_record) = get_file_metadata(link_target_action_hash(&link)?)? else {
            continue;
        };
        let file_metadata: FileMetadata = file_metadata_record.clone().try_into()?;
        if file_metadata.name == name {
            return Ok(file_metadata_record);
        }
    }

    Err(wasm_error!(WasmErrorInner::Guest("File not found".into())))
}

/// Writes a new [`FileMetadata`] version and links it from the original.
///
/// The update targets `previous_file_metadata_hash` (the current newest version) or, for a file
/// never updated, the original. Chunk handling is the caller's job.
///
/// # Errors
///
/// Host or validation errors, or `Could not find the file metadata` if the new version is not
/// readable right after being written.
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

/// Returns a link's target as an [`ActionHash`].
///
/// # Errors
///
/// `Link target is not an action hash` when the target is an entry or external hash.
pub fn link_target_action_hash(link: &Link) -> ExternResult<ActionHash> {
    link.target
        .clone()
        .into_action_hash()
        .ok_or(wasm_error!(WasmErrorInner::Guest(String::from(
            "Link target is not an action hash"
        ))))
}

/// Deletes the [`LinkTypes::PathToFileMetaData`] links from directory `fs_path` to the file whose
/// original hash is `original_file_metadata_hash`. Returns the delete-link action hashes (empty if
/// none matched).
///
/// # Errors
///
/// Host errors from `get_links` or `delete_link`.
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

/// Splits `file_content` into 1 MB [`FileChunk`]s (the last one shorter), stores each through
/// [`create_file_chunk`], and returns their entry hashes in order. Empty content yields no chunk.
///
/// # Errors
///
/// Errors from [`create_file_chunk`].
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

/// Maps a filesystem path to its DHT path string: standardize, drop the leading `/`, join the
/// segments with `.` under `root`.
///
/// ```
/// use file_system::files::fs_path_to_dht_path;
///
/// assert_eq!(fs_path_to_dht_path("/docs//reports/"), "root.docs.reports");
/// assert_eq!(fs_path_to_dht_path("/"), "root");
/// ```
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

/// Normalizes a filesystem path: backslashes become `/`, repeated separators collapse, a leading
/// `/` is added, a trailing one removed. The empty path and every all-separator path become `/`.
///
/// ```
/// use file_system::files::standardize_fs_path;
///
/// assert_eq!(standardize_fs_path("docs\\reports\\"), "/docs/reports");
/// assert_eq!(standardize_fs_path(""), "/");
/// ```
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
