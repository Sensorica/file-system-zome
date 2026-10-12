use file_system_integrity::*;
use files::*;
use hdk::prelude::*;

pub mod files;
pub mod signals;

/// A struct representing the input for creating a new file in the File Storage zome.
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct FileInput {
    pub name: String,
    pub path: String,
    pub file_type: String,
    pub content: SerializedBytes,
}

/// A struct representing the output for creating or updating a file in the File Storage zome.
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct FileOutput {
    pub file_metadata: Record,
    pub file_chunks: Vec<Record>,
}

/// Creates a new file in the File Storage zome.
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

/// Retrieves a list of file chunk records associated with the specified file metadata entry hash.
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

/// Retrieves the latest version of a file metadata entry for the specified hash.
#[hdk_extern]
pub fn get_file_metadata(original_file_metadata_hash: ActionHash) -> ExternResult<Option<Record>> {
    let latest_file_metadata_hash = latest_file_metadata_hash(&original_file_metadata_hash)?
        .unwrap_or(original_file_metadata_hash);

    get(latest_file_metadata_hash, GetOptions::default())
}

/// Retrieves all file metadata entries recursively from the specified directory path.
#[hdk_extern]
pub fn get_files_metadata_by_path_recursively(path_string: String) -> ExternResult<Vec<Record>> {
    let path_string = fs_path_to_dht_path(path_string.as_str());
    let path = Path::from(path_string);

    get_files_metadata_recursively(path)
}

/// A struct representing the input for updating a file's metadata in the File Storage zome.
#[derive(Serialize, Deserialize, SerializedBytes, Debug, Clone, PartialEq)]
pub struct UpdateFileMetadataInput {
    pub original_file_metadata_hash: ActionHash,
    pub new_content: SerializedBytes,
}

/// Updates a file by creating a new version of the file metadata entry and associating it with the previous version.
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

/// Deletes a file and all its versions, including the file chunks and its directory index link.
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
