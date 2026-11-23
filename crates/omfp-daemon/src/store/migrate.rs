//! Embedded migrations (DMN-010), applied in order using `PRAGMA user_version`, each in its own transaction with
//! foreign keys checked before commit. A new database starts from the schema of the first release (version 12);
//! databases from before it are refused.

use rusqlite::{Connection, Transaction};

use super::{Result, StoreError};

/// The schema of the first release: `user_version` 12.
const BASELINE: (i64, &str) = (12, include_str!("0001_baseline.sql"));

/// The migrations after the baseline, in order: the `n`th brings the schema to `user_version` `BASELINE.0 + n`.
const MIGRATIONS: [&str; 2] = [
    include_str!("0013_jobs.sql"),
    include_str!("0014_interrupted_turns.sql"),
];

/// The highest `user_version` this daemon knows.
const LATEST: i64 = 14;

/// Applies pending migrations. Foreign keys are off while migrating (tables are rebuilt) and checked before each
/// commit; the caller turns them on afterwards.
pub(super) fn run(conn: &mut Connection) -> Result<()> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(0..=LATEST).contains(&found) {
        return Err(StoreError::UnknownSchemaVersion {
            found,
            supported: LATEST,
        });
    }
    if (1..BASELINE.0).contains(&found) {
        return Err(StoreError::Migration(format!(
            "database schema version {found} is from a pre-release omfp; delete the database or start an older omfp"
        )));
    }
    conn.pragma_update(None, "foreign_keys", false)?;
    let baseline = (found == 0).then_some(BASELINE);
    let pending = (BASELINE.0 + 1..=LATEST)
        .zip(MIGRATIONS)
        .skip_while(|(version, _)| *version <= found);
    for (version, sql) in baseline.into_iter().chain(pending) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        check_foreign_keys(&tx)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

fn check_foreign_keys(tx: &Transaction<'_>) -> Result<()> {
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        return Err(StoreError::Migration(format!(
            "foreign key violation in table {table}"
        )));
    }
    Ok(())
}
