//! SQLite-based index for fast querying
//!
//! Provides a file_index table that indexes JSON files from the filesystem,
//! enabling fast SQL-based queries without scanning all files.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Entity types that can be indexed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Template,
    Playlist,
    Record,
    Run,  // Legacy, for backward compatibility
}

impl EntityType {
    fn as_str(&self) -> &'static str {
        match self {
            EntityType::Template => "template",
            EntityType::Playlist => "playlist",
            EntityType::Record => "record",
            EntityType::Run => "run",
        }
    }
    
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "template" => Some(EntityType::Template),
            "playlist" => Some(EntityType::Playlist),
            "record" => Some(EntityType::Record),
            "run" => Some(EntityType::Run),
            _ => None,
        }
    }
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An indexed entity from the file_index table
#[derive(Debug, Clone)]
pub struct IndexedEntity {
    pub id: String,
    pub entity_type: EntityType,
    pub name: Option<String>,
    pub file_path: PathBuf,
    pub mtime: i64,
    pub created_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub tags: Vec<String>,
    pub json_data: String,
}

/// SQLite index for runbox entities
pub struct Index {
    conn: Connection,
}

impl Index {
    /// Open or create the index database at the given path
    pub fn open(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open index database: {}", db_path.display()))?;
        
        let index = Self { conn };
        index.init_schema()?;
        
        Ok(index)
    }
    
    /// Open an in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let index = Self { conn };
        index.init_schema()?;
        Ok(index)
    }
    
    /// Initialize the database schema
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS file_index (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                name TEXT,
                file_path TEXT NOT NULL UNIQUE,
                mtime INTEGER NOT NULL,
                created_at TEXT,
                exit_code INTEGER,
                tags TEXT,
                json_data TEXT NOT NULL
            );
            
            CREATE INDEX IF NOT EXISTS idx_type ON file_index(type);
            CREATE INDEX IF NOT EXISTS idx_created_at ON file_index(created_at);
            CREATE INDEX IF NOT EXISTS idx_exit_code ON file_index(exit_code);
            
            -- Tasks table for live process tracking
            CREATE TABLE IF NOT EXISTS tasks (
                task_id TEXT PRIMARY KEY,
                record_id TEXT NOT NULL,
                runtime TEXT NOT NULL,
                status TEXT NOT NULL,
                handle_json TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                ended_at TEXT,
                exit_code INTEGER,
                log_path TEXT
            );
            
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_record ON tasks(record_id);
            "#,
        )?;
        
        Ok(())
    }
    
    /// Index a file from the filesystem
    pub fn index_file(&self, file_path: &Path, entity_type: EntityType) -> Result<()> {
        let metadata = fs::metadata(file_path)?;
        let mtime = metadata.modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        
        let json_data = fs::read_to_string(file_path)?;
        let json: serde_json::Value = serde_json::from_str(&json_data)?;
        
        // Extract common fields from JSON
        let id = self.extract_id(&json, entity_type)?;
        let name = json.get("name").and_then(|v| v.as_str()).map(String::from);
        let created_at = json.get("created_at").and_then(|v| v.as_str()).map(String::from);
        let exit_code = json.get("exit_code").and_then(|v| v.as_i64()).map(|v| v as i32);
        let tags = json.get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","))
            .unwrap_or_default();
        
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO file_index 
            (id, type, name, file_path, mtime, created_at, exit_code, tags, json_data)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                id,
                entity_type.as_str(),
                name,
                file_path.to_string_lossy(),
                mtime,
                created_at,
                exit_code,
                tags,
                json_data,
            ],
        )?;
        
        Ok(())
    }
    
    /// Extract the ID field based on entity type
    fn extract_id(&self, json: &serde_json::Value, entity_type: EntityType) -> Result<String> {
        let id_field = match entity_type {
            EntityType::Template => "template_id",
            EntityType::Playlist => "playlist_id",
            EntityType::Record => "record_id",
            EntityType::Run => "run_id",
        };
        
        json.get(id_field)
            .and_then(|v| v.as_str())
            .map(String::from)
            .with_context(|| format!("Missing {} field in JSON", id_field))
    }
    
    /// Check if a file needs re-indexing based on mtime
    pub fn needs_reindex(&self, file_path: &Path) -> Result<bool> {
        let metadata = match fs::metadata(file_path) {
            Ok(m) => m,
            Err(_) => return Ok(true), // File doesn't exist, need to reindex (or remove)
        };
        
        let current_mtime = metadata.modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        
        let stored_mtime: Option<i64> = self.conn.query_row(
            "SELECT mtime FROM file_index WHERE file_path = ?1",
            [file_path.to_string_lossy()],
            |row| row.get(0),
        ).optional()?;
        
        Ok(stored_mtime.map(|m| m != current_mtime).unwrap_or(true))
    }
    
    /// Remove a file from the index
    pub fn remove_file(&self, file_path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM file_index WHERE file_path = ?1",
            [file_path.to_string_lossy()],
        )?;
        Ok(())
    }
    
    /// Query entities with optional filters
    pub fn query(
        &self,
        entity_types: Option<&[EntityType]>,
        where_clause: Option<&str>,
        limit: usize,
    ) -> Result<Vec<IndexedEntity>> {
        let mut sql = String::from("SELECT id, type, name, file_path, mtime, created_at, exit_code, tags, json_data FROM file_index");
        let mut conditions = Vec::new();
        
        // Filter by type
        if let Some(types) = entity_types {
            let type_list: Vec<_> = types.iter().map(|t| format!("'{}'", t.as_str())).collect();
            conditions.push(format!("type IN ({})", type_list.join(",")));
        }
        
        // Custom WHERE clause
        if let Some(clause) = where_clause {
            conditions.push(format!("({})", clause));
        }
        
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        
        sql.push_str(" ORDER BY created_at DESC");
        sql.push_str(&format!(" LIMIT {}", limit));
        
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let type_str: String = row.get(1)?;
            let tags_str: String = row.get(7)?;
            
            Ok(IndexedEntity {
                id: row.get(0)?,
                entity_type: EntityType::from_str(&type_str).unwrap_or(EntityType::Run),
                name: row.get(2)?,
                file_path: PathBuf::from(row.get::<_, String>(3)?),
                mtime: row.get(4)?,
                created_at: row.get::<_, Option<String>>(5)?
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                exit_code: row.get(6)?,
                tags: if tags_str.is_empty() {
                    Vec::new()
                } else {
                    tags_str.split(',').map(String::from).collect()
                },
                json_data: row.get(8)?,
            })
        })?;
        
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("Query failed: {}", e))
    }
    
    /// Execute a raw SQL query and return results as JSON
    pub fn query_raw(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(sql)?;
        let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        
        let rows = stmt.query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, name) in column_names.iter().enumerate() {
                let value: rusqlite::types::Value = row.get(i)?;
                let json_value = match value {
                    rusqlite::types::Value::Null => serde_json::Value::Null,
                    rusqlite::types::Value::Integer(n) => serde_json::Value::Number(n.into()),
                    rusqlite::types::Value::Real(f) => {
                        serde_json::Number::from_f64(f)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null)
                    }
                    rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                    rusqlite::types::Value::Blob(b) => {
                        serde_json::Value::String(base64_encode(&b))
                    }
                };
                obj.insert(name.clone(), json_value);
            }
            Ok(serde_json::Value::Object(obj))
        })?;
        
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("Query failed: {}", e))
    }
    
    // === Task operations ===
    
    /// Save a task to the tasks table
    pub fn save_task(&self, task: &crate::Task) -> Result<()> {
        let handle_json = task.handle.as_ref()
            .map(|h| serde_json::to_string(h))
            .transpose()?;
        
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO tasks 
            (task_id, record_id, runtime, status, handle_json, created_at, started_at, ended_at, exit_code, log_path)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                task.task_id,
                task.record_id,
                task.runtime.to_string(),
                task.status.to_string(),
                handle_json,
                task.created_at.to_rfc3339(),
                task.started_at.map(|t| t.to_rfc3339()),
                task.ended_at.map(|t| t.to_rfc3339()),
                task.exit_code,
                task.log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            ],
        )?;
        
        Ok(())
    }
    
    /// Load a task by ID
    pub fn load_task(&self, task_id: &str) -> Result<Option<crate::Task>> {
        let result = self.conn.query_row(
            "SELECT task_id, record_id, runtime, status, handle_json, created_at, started_at, ended_at, exit_code, log_path FROM tasks WHERE task_id = ?1",
            [task_id],
            |row| {
                let runtime_str: String = row.get(2)?;
                let status_str: String = row.get(3)?;
                let handle_json: Option<String> = row.get(4)?;
                let created_at_str: String = row.get(5)?;
                let started_at_str: Option<String> = row.get(6)?;
                let ended_at_str: Option<String> = row.get(7)?;
                let log_path_str: Option<String> = row.get(9)?;
                
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    runtime_str,
                    status_str,
                    handle_json,
                    created_at_str,
                    started_at_str,
                    ended_at_str,
                    row.get::<_, Option<i32>>(8)?,
                    log_path_str,
                ))
            },
        ).optional()?;
        
        match result {
            Some((task_id, record_id, runtime_str, status_str, handle_json, created_at_str, started_at_str, ended_at_str, exit_code, log_path_str)) => {
                let runtime: crate::TaskRuntime = runtime_str.parse()
                    .map_err(|e: String| anyhow::anyhow!(e))?;
                let status = match status_str.as_str() {
                    "pending" => crate::TaskStatus::Pending,
                    "running" => crate::TaskStatus::Running,
                    "completed" => crate::TaskStatus::Completed,
                    "failed" => crate::TaskStatus::Failed,
                    "killed" => crate::TaskStatus::Killed,
                    _ => crate::TaskStatus::Unknown,
                };
                let handle = handle_json
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?;
                
                Ok(Some(crate::Task {
                    task_version: 0,
                    task_id,
                    record_id,
                    runtime,
                    status,
                    handle,
                    created_at: DateTime::parse_from_rfc3339(&created_at_str)?.with_timezone(&Utc),
                    started_at: started_at_str.map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc))).transpose()?,
                    ended_at: ended_at_str.map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc))).transpose()?,
                    exit_code,
                    log_path: log_path_str.map(PathBuf::from),
                }))
            }
            None => Ok(None),
        }
    }
    
    /// List tasks with optional status filter
    pub fn list_tasks(&self, status: Option<crate::TaskStatus>) -> Result<Vec<crate::Task>> {
        let sql = match status {
            Some(s) => format!(
                "SELECT task_id FROM tasks WHERE status = '{}' ORDER BY created_at DESC",
                s
            ),
            None => "SELECT task_id FROM tasks ORDER BY created_at DESC".to_string(),
        };
        
        let mut stmt = self.conn.prepare(&sql)?;
        let task_ids: Vec<String> = stmt.query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        
        let mut tasks = Vec::new();
        for task_id in task_ids {
            if let Some(task) = self.load_task(&task_id)? {
                tasks.push(task);
            }
        }
        
        Ok(tasks)
    }
    
    /// Remove a task
    pub fn remove_task(&self, task_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM tasks WHERE task_id = ?1", [task_id])?;
        Ok(())
    }
    
    /// Remove completed tasks (cleanup)
    pub fn cleanup_completed_tasks(&self) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed', 'failed', 'killed', 'unknown')",
            [],
        )?;
        Ok(deleted)
    }
}

fn base64_encode(data: &[u8]) -> String {
    // Simple hex encoding for blob data
    data.iter().map(|b| format!("{:02x}", b)).collect()}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_index_creation() {
        let index = Index::open_in_memory().unwrap();
        // Should not panic
        assert!(index.query(None, None, 10).unwrap().is_empty());
    }
    
    #[test]
    fn test_index_file() {
        let temp = tempdir().unwrap();
        let index = Index::open_in_memory().unwrap();
        
        // Create a test template file
        let template = serde_json::json!({
            "template_version": 0,
            "template_id": "tpl_test",
            "name": "Test Template",
            "exec": {
                "argv": ["echo", "hello"],
                "cwd": "."
            },
            "code_state": {
                "repo_url": "git@github.com:org/repo.git"
            }
        });
        
        let file_path = temp.path().join("tpl_test.json");
        fs::write(&file_path, serde_json::to_string_pretty(&template).unwrap()).unwrap();
        
        // Index the file
        index.index_file(&file_path, EntityType::Template).unwrap();
        
        // Query
        let results = index.query(Some(&[EntityType::Template]), None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "tpl_test");
        assert_eq!(results[0].name, Some("Test Template".to_string()));
    }
    
    #[test]
    fn test_mtime_check() {
        let temp = tempdir().unwrap();
        let index = Index::open_in_memory().unwrap();
        
        let template = serde_json::json!({
            "template_id": "tpl_test",
            "name": "Test",
            "exec": {"argv": ["echo"], "cwd": "."},
            "code_state": {"repo_url": "git@github.com:org/repo.git"}
        });
        
        let file_path = temp.path().join("tpl_test.json");
        fs::write(&file_path, serde_json::to_string(&template).unwrap()).unwrap();
        
        // Not indexed yet
        assert!(index.needs_reindex(&file_path).unwrap());
        
        // Index it
        index.index_file(&file_path, EntityType::Template).unwrap();
        
        // Should not need reindex
        assert!(!index.needs_reindex(&file_path).unwrap());
    }
    
    #[test]
    fn test_task_storage() {
        let index = Index::open_in_memory().unwrap();
        
        let mut task = crate::Task::new("rec_test".to_string(), crate::TaskRuntime::Background);
        task.mark_started(crate::TaskHandle::Background { pid: 1234, pgid: 1234 });
        
        // Save
        index.save_task(&task).unwrap();
        
        // Load
        let loaded = index.load_task(&task.task_id).unwrap().unwrap();
        assert_eq!(loaded.task_id, task.task_id);
        assert_eq!(loaded.record_id, "rec_test");
        assert_eq!(loaded.pid(), Some(1234));
        
        // List
        let tasks = index.list_tasks(Some(crate::TaskStatus::Running)).unwrap();
        assert_eq!(tasks.len(), 1);
        
        // Remove
        index.remove_task(&task.task_id).unwrap();
        assert!(index.load_task(&task.task_id).unwrap().is_none());
    }
    
    #[test]
    fn test_query_with_filter() {
        let temp = tempdir().unwrap();
        let index = Index::open_in_memory().unwrap();
        
        // Create templates
        for i in 0..3 {
            let template = serde_json::json!({
                "template_id": format!("tpl_{}", i),
                "name": format!("Template {}", i),
                "exec": {"argv": ["echo"], "cwd": "."},
                "code_state": {"repo_url": "git@github.com:org/repo.git"}
            });
            let file_path = temp.path().join(format!("tpl_{}.json", i));
            fs::write(&file_path, serde_json::to_string(&template).unwrap()).unwrap();
            index.index_file(&file_path, EntityType::Template).unwrap();
        }
        
        // Create a record
        let record = serde_json::json!({
            "record_id": "rec_test",
            "created_at": "2025-01-19T10:00:00Z",
            "exit_code": 0,
            "git_state": {"repo_url": "test", "commit": "a".repeat(40)},
            "command": {"argv": ["echo"], "cwd": "."}
        });
        let rec_path = temp.path().join("rec_test.json");
        fs::write(&rec_path, serde_json::to_string(&record).unwrap()).unwrap();
        index.index_file(&rec_path, EntityType::Record).unwrap();
        
        // Query only templates
        let templates = index.query(Some(&[EntityType::Template]), None, 10).unwrap();
        assert_eq!(templates.len(), 3);
        
        // Query only records
        let records = index.query(Some(&[EntityType::Record]), None, 10).unwrap();
        assert_eq!(records.len(), 1);
        
        // Query with custom WHERE
        let filtered = index.query(None, Some("exit_code = 0"), 10).unwrap();
        assert_eq!(filtered.len(), 1);
    }
    
    #[test]
    fn test_raw_query() {
        let index = Index::open_in_memory().unwrap();
        
        // Execute raw SQL
        let results = index.query_raw("SELECT 1 as num, 'hello' as msg").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["num"], 1);
        assert_eq!(results[0]["msg"], "hello");
    }
}
