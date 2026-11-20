use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableError};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("storage error: {0}")]
pub struct StoreError(#[from] redb::Error);

fn db_err(e: impl Into<redb::Error>) -> StoreError {
    StoreError(e.into())
}

/// Thin string-keyed byte store over redb. Each write is its own committed
/// transaction, so a crash leaves either the old or the new value. Clones
/// share one database: redb allows a file to be opened once per process.
#[derive(Clone)]
pub struct KvStore {
    db: Arc<Database>,
}

impl KvStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Database::create(path).map_err(db_err)?;
        Ok(Self { db: Arc::new(db) })
    }

    pub fn put(&self, table: &str, key: &str, value: &[u8]) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(db_err)?;
        {
            let mut t = txn.open_table(definition(table)).map_err(db_err)?;
            t.insert(key, value).map_err(db_err)?;
        }
        txn.commit().map_err(db_err)
    }

    pub fn delete(&self, table: &str, key: &str) -> Result<bool, StoreError> {
        let txn = self.db.begin_write().map_err(db_err)?;
        let removed = {
            let mut t = txn.open_table(definition(table)).map_err(db_err)?;
            t.remove(key).map_err(db_err)?.is_some()
        };
        txn.commit().map_err(db_err)?;
        Ok(removed)
    }

    pub fn clear(&self, table: &str) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(db_err)?;
        match txn.delete_table(definition(table)) {
            Ok(_) | Err(TableError::TableDoesNotExist(_)) => {}
            Err(e) => return Err(db_err(e)),
        }
        txn.commit().map_err(db_err)
    }

    pub fn get(&self, table: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = match txn.open_table(definition(table)) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(db_err(e)),
        };
        let value = t.get(key).map_err(db_err)?;
        Ok(value.map(|v| v.value().to_vec()))
    }

    pub fn values(&self, table: &str) -> Result<Vec<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read().map_err(db_err)?;
        let t = match txn.open_table(definition(table)) {
            Ok(t) => t,
            Err(TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(db_err(e)),
        };
        let mut out = Vec::new();
        for entry in t.iter().map_err(db_err)? {
            let (_, value) = entry.map_err(db_err)?;
            out.push(value.value().to_vec());
        }
        Ok(out)
    }
}

fn definition(name: &str) -> TableDefinition<'_, &'static str, &'static [u8]> {
    TableDefinition::new(name)
}
