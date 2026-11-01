//! Applied migrations are frozen.
//!
//! Databases created from these files record each file's checksum, and the only
//! thing that applies a change to an existing database is a new, higher-numbered
//! migration. Editing a file listed here would leave existing databases without
//! the change and make `sqlx` refuse to migrate them. Add a new migration instead.
//! This test needs no database.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// SHA-256 of each migration with CR removed, so a Windows checkout with autocrlf
/// hashes the same as the committed bytes.
const FROZEN: &[(&str, &str)] = &[
    ("0001_init.sql", "20551d7ba454974f7b63674df6e3bf275a953f6b39c929591e6404131c27f681"),
    ("0002_embeddings_provider_kind.sql", "5a06e94445b4361224c785b8f1865d787563b81a2c5356856f8af0ec811d1489"),
    ("0003_workspace_access.sql", "3dfa9145d1bd6e8b621fd0f6e903b3a7300756420e211a354aa7a21da37de46b"),
    ("0004_memory_lifecycle.sql", "b7b44b97135a7a7d8317ab35e82d6def1520dd6f9df7714590ffc13c5e426f3a"),
    ("0005_idempotency_ledger.sql", "f09e1be8d21d8fde2236a37b5910137fc1fb6364ec7266c59c321d39289401bc"),
    ("0006_object_version_history.sql", "c63bdf4033e40f44cde179f05cf133a9e857a4f3d63e252f9b2901b55fc0fbb1"),
];

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

#[test]
fn applied_migrations_are_unchanged() {
    for (name, expected) in FROZEN {
        let bytes = std::fs::read(migrations_dir().join(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        let normalised: Vec<u8> = bytes.into_iter().filter(|byte| *byte != b'\r').collect();
        let actual = format!("{:x}", Sha256::digest(&normalised));
        assert_eq!(
            &actual, expected,
            "{name} was already applied to databases and must not be edited; add a new migration instead"
        );
    }
}

#[test]
fn every_migration_in_the_frozen_range_is_listed() {
    let newest_frozen = FROZEN.last().expect("frozen list").0;
    let mut unlisted = Vec::new();
    for entry in std::fs::read_dir(migrations_dir()).expect("read migrations") {
        let name = entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.ends_with(".sql")
            && name.as_str() <= newest_frozen
            && !FROZEN.iter().any(|(frozen, _)| *frozen == name)
        {
            unlisted.push(name);
        }
    }
    assert_eq!(
        unlisted,
        Vec::<String>::new(),
        "a migration inside the frozen range is not in FROZEN"
    );
}
