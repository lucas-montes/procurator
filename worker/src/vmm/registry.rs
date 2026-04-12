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

//TODO: this implementation is shit
impl<F: Factory> Registry<F, Reader> {
    async fn get(self) -> OwnedRwLockReadGuard<HashMap<String, <F as Factory>::VmHandle>> {
        self.ephemeral.read_owned().await
    }

    async fn with_handle<R>(&self, id: &str, f: impl FnOnce(Option<&F::VmHandle>) -> R) -> R {
        let guard = self.ephemeral.read().await;
        f(guard.get(id))
    }
}

impl<F: Factory> Registry<F, Writer> {
    fn insert(&mut self, id: String, handle: F::VmHandle) {
        // self.ephemeral.write().unwrap().insert(id, handle);
    }

    fn delete(&mut self, id: &str) {
        // self.ephemeral.write().unwrap().remove(id);
    }
}
