//! Two-agent Sweettest scenarios for the file system zome.
//!
//! Requires the DNA bundle at `dnas/file_system/workdir/file_system.dna`; `npm test` builds it first.

use file_system::{FileInput, FileOutput, UpdateFileMetadataInput};
use file_system_integrity::{FileChunk, FileMetadata};
use holochain::prelude::*;
use holochain::sweettest::*;
use std::path::Path;

struct Scenario {
    conductors: SweetConductorBatch,
    cells: Vec<SweetCell>,
}

impl Scenario {
    /// Alice and Bob, each on their own conductor, sharing one DNA.
    async fn two_agents() -> Self {
        let mut conductors = SweetConductorBatch::standard(2).await;
        let dna_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../workdir/file_system.dna");
        let dna_file = SweetDnaFile::from_bundle(&dna_path).await.unwrap();
        let apps = conductors.setup_app("soushi-cloud", &[dna_file]).await.unwrap();
        let cells = apps.cells_flattened();
        Self { conductors, cells }
    }

    fn alice(&self) -> (&SweetConductor, SweetZome) {
        (self.conductors.get(0).unwrap(), self.cells[0].zome("file_system"))
    }

    fn bob(&self) -> (&SweetConductor, SweetZome) {
        (self.conductors.get(1).unwrap(), self.cells[1].zome("file_system"))
    }

    async fn sync(&self) {
        await_consistency(&self.cells).await.unwrap();
    }
}

fn file_input(path: &str, name: &str, content: &[u8]) -> FileInput {
    FileInput {
        name: name.to_string(),
        path: path.to_string(),
        file_type: "text/plain".to_string(),
        content: SerializedBytes::from(UnsafeBytes::from(content.to_vec())),
    }
}

fn metadata(record: &Record) -> FileMetadata {
    FileMetadata::try_from(record.clone()).unwrap()
}

fn chunk_bytes(record: &Record) -> Vec<u8> {
    FileChunk::try_from(record.clone()).unwrap().0.bytes().clone()
}

async fn create_file(conductor: &SweetConductor, zome: &SweetZome, input: FileInput) -> FileOutput {
    conductor.call(zome, "create_file", input).await
}

async fn list_recursively(conductor: &SweetConductor, zome: &SweetZome, path: &str) -> Vec<Record> {
    conductor
        .call(zome, "get_files_metadata_by_path_recursively", path.to_string())
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn create_files_and_list_them_by_path() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();
    let (bob_conductor, bob) = scenario.bob();

    for (path, name) in [
        ("/", "test.txt"),
        ("/", "index2.txt"),
        ("/subfolder", "test.txt"),
        ("/subfolder2", "index2.txt"),
        ("/subfolder/subfolder3", "test.txt"),
    ] {
        create_file(alice_conductor, &alice, file_input(path, name, b"hello world !")).await;
    }

    scenario.sync().await;

    // Bob sees Alice's files, each directory listing including its subdirectories.
    assert_eq!(list_recursively(bob_conductor, &bob, "/").await.len(), 5);
    assert_eq!(list_recursively(bob_conductor, &bob, "/subfolder").await.len(), 2);
    assert_eq!(list_recursively(bob_conductor, &bob, "/subfolder2").await.len(), 1);

    let deepest = list_recursively(bob_conductor, &bob, "/subfolder/subfolder3").await;
    assert_eq!(deepest.len(), 1);
    let file = metadata(&deepest[0]);
    assert_eq!(file.name, "test.txt");
    assert_eq!(file.path, "/subfolder/subfolder3");
    assert_eq!(file.author, scenario.cells[0].agent_pubkey().clone());
}

#[tokio::test(flavor = "multi_thread")]
async fn large_file_is_chunked_and_deleted() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();
    let (bob_conductor, bob) = scenario.bob();

    // 5 MB with distinct bytes per megabyte, so each chunk is its own entry.
    let content: Vec<u8> = (0..5 * 1024 * 1024).map(|i| (i / (1024 * 1024)) as u8).collect();
    let output = create_file(alice_conductor, &alice, file_input("/", "large_file.txt", &content)).await;
    assert_eq!(output.file_chunks.len(), 5);
    assert_eq!(metadata(&output.file_metadata).size, content.len());

    let original_hash = output.file_metadata.action_address().clone();
    scenario.sync().await;

    // Bob reassembles the file from its chunks.
    let chunks: Vec<Record> = bob_conductor.call(&bob, "get_file_chunks", original_hash.clone()).await;
    let reassembled: Vec<u8> = chunks.iter().flat_map(chunk_bytes).collect();
    assert_eq!(reassembled, content);

    let _: Vec<ActionHash> = alice_conductor.call(&alice, "delete_file", original_hash.clone()).await;
    scenario.sync().await;

    let read: Option<Record> = bob_conductor.call(&bob, "get_file_metadata", original_hash.clone()).await;
    assert!(read.is_none());
    for chunk in &output.file_chunks {
        let chunk_read: Option<Record> = bob_conductor
            .call(&bob, "get_file_metadata", chunk.action_address().clone())
            .await;
        assert!(chunk_read.is_none(), "chunk should be deleted with its file");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn update_file_twice_then_delete_in_cascade() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();
    let (bob_conductor, bob) = scenario.bob();

    let output = create_file(alice_conductor, &alice, file_input("/", "test.txt", b"hello world !")).await;
    let original_hash = output.file_metadata.action_address().clone();
    assert_eq!(output.file_chunks.len(), 1);

    scenario.sync().await;

    // A second file with the same path and name is refused.
    let duplicate: Result<FileOutput, _> = alice_conductor
        .call_fallible(&alice, "create_file", file_input("/", "test.txt", b"new content"))
        .await;
    assert!(duplicate.is_err());

    // First update: Bob reads the new content through the original hash.
    let _: FileOutput = alice_conductor
        .call(
            &alice,
            "update_file",
            UpdateFileMetadataInput {
                original_file_metadata_hash: original_hash.clone(),
                new_content: SerializedBytes::from(UnsafeBytes::from(b"new content".to_vec())),
            },
        )
        .await;
    scenario.sync().await;
    let chunks: Vec<Record> = bob_conductor.call(&bob, "get_file_chunks", original_hash.clone()).await;
    assert_eq!(chunk_bytes(&chunks[0]), b"new content");

    // Second update: the version chain still resolves to the newest one.
    let second: FileOutput = alice_conductor
        .call(
            &alice,
            "update_file",
            UpdateFileMetadataInput {
                original_file_metadata_hash: original_hash.clone(),
                new_content: SerializedBytes::from(UnsafeBytes::from(b"new content 2".to_vec())),
            },
        )
        .await;
    scenario.sync().await;
    let chunks: Vec<Record> = bob_conductor.call(&bob, "get_file_chunks", original_hash.clone()).await;
    assert_eq!(chunk_bytes(&chunks[0]), b"new content 2");
    let latest: Option<Record> = bob_conductor.call(&bob, "get_file_metadata", original_hash.clone()).await;
    assert_eq!(latest.unwrap().action_address(), second.file_metadata.action_address());

    // Deleting through the original hash removes every version and its chunks.
    let _: Vec<ActionHash> = alice_conductor.call(&alice, "delete_file", original_hash.clone()).await;
    scenario.sync().await;

    let second_hash = second.file_metadata.action_address().clone();
    for hash in [original_hash, second_hash] {
        let read: Option<Record> = bob_conductor.call(&bob, "get_file_metadata", hash.clone()).await;
        assert!(read.is_none());
        let chunks: Vec<Record> = bob_conductor.call(&bob, "get_file_chunks", hash).await;
        assert!(chunks.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn validation_and_path_standardization() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();

    // Validation rejects an empty name and a path holding forbidden characters.
    let empty_name: Result<FileOutput, _> = alice_conductor
        .call_fallible(&alice, "create_file", file_input("/", "", b"x"))
        .await;
    assert!(empty_name.unwrap_err().to_string().contains("File name cannot be empty"));

    let forbidden: Result<FileOutput, _> = alice_conductor
        .call_fallible(&alice, "create_file", file_input("/?.34", "test.txt", b"x"))
        .await;
    assert!(forbidden
        .unwrap_err()
        .to_string()
        .contains("File path cannot contain forbidden characters"));

    // '/' and '\' both mean the root, and repeated separators collapse.
    let slash = create_file(alice_conductor, &alice, file_input("/", "test.txt", b"x")).await;
    let backslash = create_file(alice_conductor, &alice, file_input("\\", "test2.txt", b"x")).await;
    assert_eq!(metadata(&slash.file_metadata).path, "/");
    assert_eq!(metadata(&backslash.file_metadata).path, "/");

    let nested = create_file(
        alice_conductor,
        &alice,
        file_input("/subfolder///subfolder2", "test3.txt", b"x"),
    )
    .await;
    assert_eq!(metadata(&nested.file_metadata).path, "/subfolder/subfolder2");
}

#[tokio::test(flavor = "multi_thread")]
async fn deleted_file_leaves_its_directory_listing() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();
    let (bob_conductor, bob) = scenario.bob();

    let output = create_file(alice_conductor, &alice, file_input("/docs", "report.txt", b"v1")).await;
    create_file(alice_conductor, &alice, file_input("/docs", "other.txt", b"kept")).await;
    scenario.sync().await;
    assert_eq!(list_recursively(bob_conductor, &bob, "/docs").await.len(), 2);

    let _: Vec<ActionHash> = alice_conductor
        .call(&alice, "delete_file", output.file_metadata.action_address().clone())
        .await;
    scenario.sync().await;

    let remaining = list_recursively(bob_conductor, &bob, "/docs").await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(metadata(&remaining[0]).name, "other.txt");

    // The name is free again once the file is gone.
    let recreated = create_file(alice_conductor, &alice, file_input("/docs", "report.txt", b"v2")).await;
    assert_eq!(metadata(&recreated.file_metadata).name, "report.txt");
}

#[tokio::test(flavor = "multi_thread")]
async fn file_metadata_changes_emit_signals() {
    let scenario = Scenario::two_agents().await;
    let (alice_conductor, alice) = scenario.alice();
    let mut signals = alice_conductor.subscribe_to_app_signals("soushi-cloud".to_string());

    let output = create_file(alice_conductor, &alice, file_input("/", "signals.txt", b"v1")).await;
    let original_hash = output.file_metadata.action_address().clone();
    let _: FileOutput = alice_conductor
        .call(
            &alice,
            "update_file",
            UpdateFileMetadataInput {
                original_file_metadata_hash: original_hash.clone(),
                new_content: SerializedBytes::from(UnsafeBytes::from(b"v2".to_vec())),
            },
        )
        .await;
    let _: Vec<ActionHash> = alice_conductor.call(&alice, "delete_file", original_hash).await;

    // Chunk actions emit nothing; only file metadata does, in commit order.
    let mut kinds = Vec::new();
    while let Ok(Ok(signal)) =
        tokio::time::timeout(std::time::Duration::from_secs(5), signals.recv()).await
    {
        if let holochain::prelude::Signal::App { signal, .. } = signal {
            let kind = match signal.into_inner().decode::<file_system::signals::Signal>().unwrap() {
                file_system::signals::Signal::FileMetadataCreated { .. } => "created",
                file_system::signals::Signal::FileMetadataUpdated { .. } => "updated",
                file_system::signals::Signal::FileMetadataDeleted { .. } => "deleted",
            };
            if kinds.last() != Some(&kind) {
                kinds.push(kind);
            }
        }
        if kinds.last() == Some(&"deleted") {
            break;
        }
    }
    assert_eq!(kinds, vec!["created", "updated", "deleted"]);
}
