# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- Ported to Holochain 0.7: `hdk 0.7.0`, `hdi 0.8.0`, holonix `main-0.7`, Node 24. A 0.7 DNA is a new network, so data from the 0.1 proof of concept does not carry over.
- Validation uses the 0.7 `FlatOp` shape; signals match on `ActionData`; links are read with `LinkQuery` and `GetStrategy`.
- DNA and hApp manifests moved to the 0.7 format.
- Tests moved from Tryorama to Sweettest, eight two-agent scenarios run by `npm test`, signals included. Tryorama has no Holochain 0.7 release.
- `npm start` runs a local bootstrap and relay server with two conductors.

### Fixed

- Deleting a file now removes its directory index link, so it leaves directory listings and its name can be reused.
- Deleting a file that was never updated now deletes its chunks too.
- Updating a file no longer fails when the new version reuses a chunk of the old one (re-saving the same content, appending to a file over 1 MB).
- `update_file` and `delete_file` reject a later version's hash instead of leaving the directory index pointing at a deleted version.
- A directory link whose file is gone no longer blocks the duplicate-name check.
- Zome calls return an error instead of panicking when a chunk cannot be read.
- Removed debug `println!` and `warn!` output from path handling.

### Removed

- `workdir/web-happ.yaml` and the `package` script, which pointed at a `ui/` folder removed earlier.
