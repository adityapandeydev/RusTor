//! Retain exclusively-created filesystem handles for native error recovery.
//! librqbit retries call the factory again; create-new mode would otherwise
//! reject the payloads that this torrent itself created. No path is reopened.
use anyhow::{Context, Result, ensure};
use librqbit::{
    ManagedTorrentShared, TorrentMetadata,
    storage::{
        BoxStorageFactory, StorageFactory, StorageFactoryExt, TorrentStorage,
        filesystem::FilesystemStorageFactory,
    },
};
use std::{
    any::TypeId,
    io::IoSlice,
    path::Path,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub(crate) struct RetryStorageFactory {
    files: Arc<Mutex<Option<Arc<Files>>>>,
}
struct Files {
    // A new generation invalidates old wrappers and drains their in-flight I/O.
    inner: RwLock<(u64, Box<dyn TorrentStorage>)>,
}
pub(crate) struct Storage {
    files: Arc<Files>,
    generation: u64,
    active: AtomicBool,
}
impl StorageFactory for RetryStorageFactory {
    type Storage = Storage;
    fn create(&self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> Result<Storage> {
        let mut slot = self
            .files
            .lock()
            .map_err(|_| anyhow::anyhow!("storage factory lock poisoned"))?;
        let files = match slot.as_ref() {
            Some(files) => files.clone(),
            None => {
                // Native create-new protection applies on the first and only open.
                let storage =
                    FilesystemStorageFactory::default().create_and_init(shared, metadata)?;
                let files = Arc::new(Files {
                    inner: RwLock::new((0, Box::new(storage))),
                });
                *slot = Some(files.clone());
                files
            }
        };
        let generation = {
            let mut inner = files
                .inner
                .write()
                .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
            inner.0 = inner
                .0
                .checked_add(1)
                .context("storage generation exhausted")?;
            inner.0
        };
        Ok(Storage {
            files,
            generation,
            active: AtomicBool::new(true),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
    fn is_type_id(&self, id: TypeId) -> bool {
        // The native JSON persistence must still recognize this as filesystem data.
        id == TypeId::of::<Self>() || id == TypeId::of::<FilesystemStorageFactory>()
    }
}
impl Storage {
    fn with<R>(&self, operation: impl FnOnce(&dyn TorrentStorage) -> Result<R>) -> Result<R> {
        let inner = self
            .files
            .inner
            .read()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        ensure!(
            self.active.load(Ordering::Acquire) && inner.0 == self.generation,
            "storage handle is no longer active"
        );
        operation(inner.1.as_ref())
    }
}
impl TorrentStorage for Storage {
    fn init(&mut self, _: &ManagedTorrentShared, _: &TorrentMetadata) -> Result<()> {
        Ok(())
    }
    fn pread_exact(&self, file: usize, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.with(|s| s.pread_exact(file, offset, buf))
    }
    fn pwrite_all(&self, file: usize, offset: u64, buf: &[u8]) -> Result<()> {
        #[cfg(test)]
        if FAIL_WRITES.load(Ordering::Acquire) {
            anyhow::bail!("controlled storage write failure");
        }
        self.with(|s| s.pwrite_all(file, offset, buf))
    }
    fn pwrite_all_vectored(
        &self,
        file: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> Result<usize> {
        #[cfg(test)]
        if FAIL_WRITES.load(Ordering::Acquire) {
            anyhow::bail!("controlled storage write failure");
        }
        self.with(|s| s.pwrite_all_vectored(file, offset, bufs))
    }
    fn ensure_file_length(&self, file: usize, length: u64) -> Result<()> {
        self.with(|s| s.ensure_file_length(file, length))
    }
    fn remove_file(&self, file: usize, name: &Path) -> Result<()> {
        self.with(|s| s.remove_file(file, name))
    }
    fn remove_directory_if_empty(&self, path: &Path) -> Result<()> {
        self.with(|s| s.remove_directory_if_empty(path))
    }
    fn take(&self) -> Result<Box<dyn TorrentStorage>> {
        // Pausing must drain old I/O before handing the storage to its new owner.
        let inner = self
            .files
            .inner
            .write()
            .map_err(|_| anyhow::anyhow!("storage lock poisoned"))?;
        ensure!(
            inner.0 == self.generation && self.active.swap(false, Ordering::AcqRel),
            "storage handle is no longer active"
        );
        Ok(Box::new(Storage {
            files: self.files.clone(),
            generation: self.generation,
            active: AtomicBool::new(true),
        }))
    }
}

#[cfg(test)]
pub(crate) static FAIL_WRITES: AtomicBool = AtomicBool::new(false);
