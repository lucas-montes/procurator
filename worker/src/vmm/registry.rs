use std::sync::Arc;

use sqlx::SqlitePool;

use crate::database::Database;
use crate::vmm::interfaces::Factory;
use std::collections::HashMap;
use std::marker::PhantomData;
use tokio::sync::{OwnedRwLockReadGuard, RwLock};

/// Type markers for registry access control.
#[derive(Clone)]
pub struct Writer;

#[derive(Clone)]
pub struct Reader;

pub struct Uninitialized;

/// Registry holding VM handles with typed access control.
pub struct Registry<F: Factory, S = Uninitialized> {
    persistent: SqlitePool,
    ephemeral: Arc<RwLock<HashMap<String, F::VmHandle>>>,
    _side: PhantomData<S>,
}

impl<F: Factory> Registry<F, Uninitialized> {
    pub fn new(persistent: SqlitePool) -> Self {
        Self {
            persistent,
            ephemeral: Arc::new(RwLock::new(HashMap::new())),
            _side: PhantomData,
        }
    }

    pub fn split(self) -> (Registry<F, Reader>, Registry<F, Writer>) {
        (
            Registry {
                persistent: self.persistent.clone(),
                ephemeral: self.ephemeral.clone(),
                _side: PhantomData,
            },
            Registry {
                persistent: self.persistent,
                ephemeral: self.ephemeral,
                _side: PhantomData,
            },
        )
    }
}

impl<F: Factory, S> Clone for Registry<F, S> {
    fn clone(&self) -> Self {
        Self {
            persistent: self.persistent.clone(),
            ephemeral: self.ephemeral.clone(),
            _side: PhantomData,
        }
    }
}

impl<F: Factory> Registry<F, Side> {
    pub async fn get(self) -> OwnedRwLockReadGuard<HashMap<String, <F as Factory>::VmHandle>> {
        self.ephemeral.read_owned().await
    }
}

impl<F: Factory> Registry<F, Reader> {
    pub async fn exists(&self, id: &str) -> bool {
        self.ephemeral.read().await.contains_key(id)
    }
}

impl<F: Factory> Registry<F, Writer> {
    pub async fn insert(&mut self, id: String, handle: F::VmHandle) {
        self.ephemeral.write().await.insert(id, handle);
    }

    pub async fn remove(&mut self, id: &str) -> std::option::Option<F::VmHandle> {
        self.ephemeral.write().await.remove(id)
    }
}
