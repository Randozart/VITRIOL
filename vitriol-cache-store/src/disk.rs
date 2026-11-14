use crate::model::Ledger;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    pub bytes: u64,
}

impl FileIdentity {
    fn read(path: &Path) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("managed file is not regular".into());
        }
        Ok(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            bytes: metadata.len(),
        })
    }
}

pub struct Directory {
    root: PathBuf,
    _lock: File,
}

impl Directory {
    pub fn open(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .map_err(|e| e.to_string())?;
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err("managed namespace must be a private regular directory".into());
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path.join("owner.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|e| format!("managed namespace already owned or unavailable: {e}"))?;
        let pending = path.join("pending");
        if !pending.exists() {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&pending)
                .map_err(|e| e.to_string())?;
        }
        if !fs::symlink_metadata(&pending)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("pending namespace is not a directory".into());
        }
        Ok(Self {
            root: path.to_owned(),
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn stage_path(&self, ticket: u64) -> PathBuf {
        self.root
            .join("pending")
            .join(format!("{ticket:016x}"))
            .join("snapshot.bin")
    }

    pub fn load(&self) -> Result<Ledger, String> {
        let path = self.root.join("ledger.json");
        if !path.exists() {
            return Ok(Ledger::default());
        }
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("ledger is not a regular file".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        ledger.validate()?;
        Ok(ledger)
    }

    /// Local POSIX fsync and atomic rename form the durable metadata acknowledgment.
    pub fn save(&self, ledger: &Ledger) -> Result<(), String> {
        ledger.validate()?;
        let path = self.root.join("ledger.next");
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(ledger).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(path, self.root.join("ledger.json")).map_err(|e| e.to_string())?;
        self.sync_root()
    }

    pub fn reserve_stage(&self, ticket: u64) -> Result<(), String> {
        let parent = self.stage_path(ticket).parent().unwrap().to_owned();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(parent)
            .map_err(|e| e.to_string())
    }

    pub fn staged_identity(&self, ticket: u64) -> Result<FileIdentity, String> {
        FileIdentity::read(&self.stage_path(ticket))
    }

    pub fn publish(&self, ticket: u64, id: &str) -> Result<(), String> {
        let stage = self.stage_path(ticket);
        File::open(&stage)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        // A link cannot overwrite a pre-existing incarnation. Both paths are on the same filesystem.
        fs::hard_link(&stage, self.root.join(id)).map_err(|e| e.to_string())?;
        self.sync_root()?;
        fs::remove_dir_all(stage.parent().unwrap()).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn verify(&self, id: &str, expected: &FileIdentity) -> Result<(), String> {
        if &FileIdentity::read(&self.root.join(id))? != expected {
            return Err("managed file incarnation changed".into());
        }
        Ok(())
    }

    /// Only ledger-owned abandoned tickets/tombstones are eligible for recovery cleanup.
    pub fn recover(&self, ledger: &mut Ledger) -> Result<(), String> {
        for (id, record) in &ledger.tombstones {
            let path = self.root.join(id);
            if path.exists() {
                self.verify(id, &record.identity)?;
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        for (ticket, pending) in &ledger.abandoned {
            let stage = self.stage_path(*ticket);
            if stage.parent().unwrap().exists() {
                fs::remove_dir_all(stage.parent().unwrap()).map_err(|e| e.to_string())?;
            }
            let path = self.root.join(&pending.id);
            if let Some(identity) = &pending.file_identity {
                if path.exists() {
                    self.verify(&pending.id, identity)?;
                    fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
        }
        self.sync_root()?;
        ledger.tombstones.clear();
        ledger.abandoned.clear();
        self.save(ledger)
    }

    fn sync_root(&self) -> Result<(), String> {
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
}
