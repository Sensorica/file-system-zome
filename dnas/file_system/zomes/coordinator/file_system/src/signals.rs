//! Local signals telling an open app that a file changed, so it can refresh without polling.
//!
//! Signals go to the clients connected to the agent's own conductor, after the agent's own
//! commits; they are not sent to other peers. Only [`FileMetadata`] actions emit; chunk and link
//! actions are silent. Serialized with `serde(tag = "type")`, so a JavaScript client receives
//! `{ type: "FileMetadataCreated", action, app_entry }` and so on.

use file_system_integrity::*;
use hdk::prelude::*;

/// A change to a file, emitted once per committed [`FileMetadata`] action.
///
/// One [`delete_file`](crate::delete_file) call emits one `FileMetadataDeleted` per version it
/// deletes.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Signal {
    /// A file was created by [`create_file`](crate::create_file).
    FileMetadataCreated {
        /// The create action; its hash is the file's original hash.
        action: SignedActionHashed,
        /// The new [`EntryTypes::FileMetadata`].
        app_entry: EntryTypes,
    },
    /// A file got a new version from [`update_file`](crate::update_file).
    FileMetadataUpdated {
        /// The update action that wrote the new version.
        action: SignedActionHashed,
        /// The new version's [`EntryTypes::FileMetadata`].
        app_entry: EntryTypes,
        /// The version this one replaces (the previous newest, not necessarily the first).
        original_app_entry: EntryTypes,
    },
    /// A version of a file was deleted by [`delete_file`](crate::delete_file).
    FileMetadataDeleted {
        /// The delete action.
        action: SignedActionHashed,
        /// The [`EntryTypes::FileMetadata`] version that was deleted.
        original_app_entry: EntryTypes,
    },
}

/// Called by the conductor after each successful commit; emits a [`Signal`] for every
/// [`FileMetadata`] action in it.
///
/// Infallible by contract: a failure to build or emit one signal is logged and the others are
/// still sent.
#[hdk_extern(infallible)]
pub fn post_commit(committed_actions: Vec<SignedActionHashed>) {
    for action in committed_actions {
        if let Err(err) = signal_action(action) {
            error!("Error signaling new action: {:?}", err);
        }
    }
}

/// Emits the [`Signal`] matching one committed action, or nothing if it is not about a file.
fn signal_action(action: SignedActionHashed) -> ExternResult<()> {
    match &action.hashed.content.clone().data {
        ActionData::Create(_create) => {
            if let Some(app_entry @ EntryTypes::FileMetadata(_)) =
                get_entry_for_action(&action.hashed.hash)?
            {
                emit_signal(&Signal::FileMetadataCreated { action, app_entry })?;
            }
            Ok(())
        }
        ActionData::Update(update) => {
            if let Some(app_entry @ EntryTypes::FileMetadata(_)) =
                get_entry_for_action(&action.hashed.hash)?
            {
                if let Some(original_app_entry) =
                    get_entry_for_action(&update.original_action_address)?
                {
                    emit_signal(&Signal::FileMetadataUpdated {
                        action,
                        app_entry,
                        original_app_entry,
                    })?;
                }
            }
            Ok(())
        }
        ActionData::Delete(delete) => {
            if let Some(original_app_entry @ EntryTypes::FileMetadata(_)) =
                get_entry_for_action(&delete.deletes_address)?
            {
                emit_signal(&Signal::FileMetadataDeleted {
                    action,
                    original_app_entry,
                })?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Fetches the app entry written by `action_hash`, or `None` if the action carries no entry of
/// this zome.
fn get_entry_for_action(action_hash: &ActionHash) -> ExternResult<Option<EntryTypes>> {
    let record = match get_details(action_hash.clone(), GetOptions::default())? {
        Some(Details::Record(record_details)) => record_details.record,
        _ => {
            return Ok(None);
        }
    };
    let entry = match record.entry().as_option() {
        Some(entry) => entry,
        None => {
            return Ok(None);
        }
    };
    let (zome_index, entry_index) = match record.action().entry_type() {
        Some(EntryType::App(AppEntryDef {
            zome_index,
            entry_index,
            ..
        })) => (zome_index, entry_index),
        _ => {
            return Ok(None);
        }
    };
    EntryTypes::deserialize_from_type(*zome_index, *entry_index, entry)
}
