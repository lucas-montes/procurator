use super::interfaces::Factory;
use crate::database::Database;
use std::{collections::HashMap, marker::PhantomData, sync::Arc};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};

/// Type to identify what the registry is allowed to do.
#[derive(Clone)]
pub struct Writer;

#[derive(Clone)]
pub struct Reader;

pub struct Unitiliazed;

/// The State holding the information about what is running and other persistent stuff that I need to think about.
/// How could I have some typeshit that would prevent writing into the hashmap?
/// MAybe the registry could hold the factory?
pub struct Registry<F: Factory, Side = Unitiliazed> {
    persistent: Database,
    ephemeral: Arc<RwLock<HashMap<String, F::VmHandle>>>,
    _side: PhantomData<Side>,
}

impl<F: Factory> Registry<F, Unitiliazed> {
    pub fn new(persistent: Database) -> Self {
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
                persistent: self.persistent.clone(),
                ephemeral: self.ephemeral.clone(),
                _side: PhantomData,
            },
        )
    }
}

impl<F: Factory, Side> Clone for Registry<F, Side> {
    fn clone(&self) -> Self {
        Self {
            persistent: self.persistent.clone(),
            ephemeral: self.ephemeral.clone(),
            _side: PhantomData,
        }
    }
}
impl<F: Factory, Side> Registry<F, Side> {
    pub async fn get(self) -> OwnedRwLockReadGuard<HashMap<String, <F as Factory>::VmHandle>> {
        //TODO: this might block until the writer is released, so a lot of quick writes could be bad
        self.ephemeral.read_owned().await
    }
}

//TODO: this implementation is shit
impl<F: Factory> Registry<F, Reader> {
    pub async fn exists(&self, id: &str) -> bool {
        self.ephemeral.read().await.contains_key(id)
    }
}

impl<F: Factory> Registry<F, Writer> {
    pub async fn insert(&mut self, id: String, handle: F::VmHandle) {
        self.ephemeral.write().await.insert(id, handle);
    }

    pub async fn remove(&mut self, id: &str) -> std::option::Option<<F as Factory>::VmHandle> {
        self.ephemeral.write().await.remove(id)
    }
}
