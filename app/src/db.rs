use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

pub struct Store(Mutex<Connection>);

impl Store {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        Self::initialize(conn)
    }
    #[cfg(test)]
    pub fn memory() -> Result<Self, rusqlite::Error> {
        Self::initialize(Connection::open_in_memory()?)
    }
    fn initialize(conn: Connection) -> Result<Self, rusqlite::Error> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))); PRAGMA user_version = 1; COMMIT;")?;
        Ok(Self(Mutex::new(conn)))
    }
    pub fn list(&self) -> Result<Vec<Project>, rusqlite::Error> {
        let conn = self.0.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
        let mut stmt = conn.prepare(
            "SELECT id, name, created_at FROM projects ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        rows.collect()
    }
    pub fn get(&self, id: &str) -> Result<Option<Project>, rusqlite::Error> {
        let conn = self.0.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
        conn.query_row(
            "SELECT id, name, created_at FROM projects WHERE id = ?1",
            [id],
            row_to_project,
        )
        .optional()
    }
    pub fn delete(&self, id: &str) -> Result<bool, rusqlite::Error> {
        let conn = self.0.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(conn.execute("DELETE FROM projects WHERE id = ?1", [id])? > 0)
    }
    /// Build a standalone snapshot under the connection lock. Do not wait behind an active write.
    pub fn export(&self) -> Result<Vec<u8>, ExportError> {
        let conn = self.0.try_lock().map_err(|_| ExportError::Busy)?;
        let temp = tempfile::tempdir().map_err(|_| ExportError::Storage)?;
        let path = temp.path().join("StudioLine-export.db");
        let path_str = path.to_str().ok_or(ExportError::Storage)?;
        conn.execute("VACUUM main INTO ?1", [path_str])
            .map_err(|_| ExportError::Storage)?;
        std::fs::read(path).map_err(|_| ExportError::Storage)
    }
    pub fn create(&self, name: &str) -> Result<Project, CreateError> {
        let name = validate_name(name).map_err(CreateError::Invalid)?;
        let id = Uuid::new_v4().to_string();
        let mut conn = self
            .0
            .lock()
            .map_err(|_| CreateError::Database(rusqlite::Error::InvalidQuery))?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut stmt = transaction.prepare("SELECT name FROM projects")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for existing in rows {
                if caseless::canonical_caseless_match_str(existing?.trim(), name) {
                    return Err(CreateError::Duplicate);
                }
            }
        }
        transaction.execute(
            "INSERT INTO projects (id, name) VALUES (?1, ?2)",
            params![id, name],
        )?;
        let project = transaction.query_row(
            "SELECT id, name, created_at FROM projects WHERE id = ?1",
            [&id],
            row_to_project,
        )?;
        transaction.commit()?;
        Ok(project)
    }
}
#[derive(Debug)]
pub enum ExportError {
    Busy,
    Storage,
}

#[derive(Debug)]
pub enum CreateError {
    Duplicate,
    Invalid(&'static str),
    Database(rusqlite::Error),
}
impl From<rusqlite::Error> for CreateError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

fn row_to_project(row: &rusqlite::Row<'_>) -> Result<Project, rusqlite::Error> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
    })
}

pub fn validate_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a project name.");
    }
    if name.chars().count() > 80 {
        return Err("Project names must be 80 characters or fewer.");
    }
    if name.chars().any(char::is_control) {
        return Err("Project names cannot contain control characters.");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_names_and_trims_valid_names() {
        assert!(validate_name(" \n ").is_err());
        assert!(validate_name("a\u{0000}b").is_err());
        assert!(validate_name(&"x".repeat(81)).is_err());
        assert_eq!(validate_name("  Village  "), Ok("Village"));
        assert!(validate_name(&"界".repeat(80)).is_ok());
    }
    #[test]
    fn saves_and_reads_projects() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projects.sqlite");
        let project = Store::open(&path).unwrap().create("Village").unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.get(&project.id).unwrap(), Some(project.clone()));
        assert_eq!(reopened.list().unwrap(), vec![project]);
        assert_eq!(reopened.get("missing").unwrap(), None);
    }
    #[test]
    fn duplicate_names_are_rejected_and_old_duplicates_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("old.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE projects (id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))); INSERT INTO projects VALUES ('1','Test','a'); INSERT INTO projects VALUES ('2','TEST','b'); PRAGMA user_version = 1;").unwrap();
        drop(conn);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(matches!(
            store.create(" test "),
            Err(CreateError::Duplicate)
        ));
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.create("Other").is_ok());
    }
    #[test]
    fn concurrent_stores_cannot_create_the_same_name() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("concurrent.sqlite");
        let first = Store::open(&path).unwrap();
        let second = Store::open(&path).unwrap();
        let a = std::thread::spawn(move || first.create("Test"));
        let b = std::thread::spawn(move || second.create(" test "));
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(CreateError::Duplicate)))
                .count(),
            1
        );
        assert_eq!(Store::open(&path).unwrap().list().unwrap().len(), 1);
    }
    #[test]
    fn unicode_case_and_normalization_match() {
        let store = Store::memory().unwrap();
        store.create("Straße").unwrap();
        assert!(matches!(
            store.create("STRASSE"),
            Err(CreateError::Duplicate)
        ));
        store.create("Café").unwrap();
        assert!(matches!(
            store.create("CAFE\u{301}"),
            Err(CreateError::Duplicate)
        ));
    }
    #[test]
    fn export_is_a_readable_standalone_snapshot_and_does_not_change_after_delete() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("live.db")).unwrap();
        let project = store.create("Keep in export").unwrap();
        let bytes = store.export().unwrap();
        let backup = temp.path().join("backup.db");
        std::fs::write(&backup, bytes).unwrap();
        assert!(store.delete(&project.id).unwrap());
        assert!(!store.delete(&project.id).unwrap());
        let exported = Store::open(&backup).unwrap();
        assert_eq!(exported.list().unwrap(), vec![project]);
        assert!(store.list().unwrap().is_empty());
    }
    #[test]
    fn export_fails_busy_instead_of_waiting_behind_a_write() {
        let store = Store::memory().unwrap();
        let _active = store.0.lock().unwrap();
        assert!(matches!(store.export(), Err(ExportError::Busy)));
    }
    #[test]
    fn newer_database_schema_is_not_modified() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("new.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        assert!(Store::open(&path).is_err());
        let conn = Connection::open(path).unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 99);
    }
}
