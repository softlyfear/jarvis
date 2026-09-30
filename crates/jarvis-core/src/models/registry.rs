use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::{Mutex, RwLock};

use super::structs::ModelDef;

thread_local! {
    static LOAD_STACK: std::cell::RefCell<Vec<(usize, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}
struct LoadGuard;
impl LoadGuard {
    fn enter(registry: usize, id: &str) -> Result<Self, String> {
        LOAD_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.iter().any(|(r, name)| *r == registry && name == id) {
                return Err(format!("Cyclic dependency while loading model '{}'", id));
            }
            stack.push((registry, id.to_string()));
            Ok(Self)
        })
    }
}
impl Drop for LoadGuard {
    fn drop(&mut self) { LOAD_STACK.with(|stack| { stack.borrow_mut().pop(); }); }
}

// central model registry. loads models once and shares them between components.
// completely type-agnostic
pub struct ModelRegistry {
    loaded: Mutex<HashMap<String, Arc<dyn Any + Send + Sync>>>,
    loading: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    catalog: RwLock<Vec<ModelDef>>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            loaded: Mutex::new(HashMap::new()),
            loading: Mutex::new(HashMap::new()),
            catalog: RwLock::new(Vec::new()),
        }
    }

    pub fn set_catalog(&self, defs: Vec<ModelDef>) {
        *self.catalog.write() = defs;
    }

    // read access to catalog without cloning the whole vec
    pub fn with_catalog<R>(&self, f: impl FnOnce(&[ModelDef]) -> R) -> R {
        f(&self.catalog.read())
    }

    pub fn get_model_def(&self, id: &str) -> Option<ModelDef> {
        self.catalog.read().iter().find(|m| m.id == id).cloned()
    }

    // get a loaded model, downcasted to the expected type
    pub fn get<T: 'static + Send + Sync>(&self, id: &str) -> Option<Arc<T>> {
        self.loaded.lock()
            .get(id)?
            .clone()
            .downcast::<T>()
            .ok()
    }

    // get or load a model. if two components request the same id,
    // the model only loads once.
    //
    // the global lock is released before calling the loader to avoid deadlocks
    // if the loader tries to load a dependency through the registry.
    pub fn get_or_load<T: 'static + Send + Sync>(
        &self,
        id: &str,
        loader: impl FnOnce(&ModelDef) -> Result<T, String>,
    ) -> Result<Arc<T>, String> {
        // fast path: already loaded
        if let Some(existing) = self.get::<T>(id) {
            info!("Model '{}' already loaded, reusing", id);
            return Ok(existing);
        }

        // Serialize expensive loads per id; loaders can still request other model ids.
        let _load_stack = LoadGuard::enter(self as *const Self as usize, id)?;
        let gate = self.loading.lock().entry(id.to_string()).or_default().clone();
        let _loading = gate.lock();
        if let Some(existing) = self.loaded.lock().get(id).cloned() {
            return existing.downcast::<T>().map_err(|_| format!("Model '{}' already has a different type", id));
        }

        // grab model def (releases catalog lock immediately)
        let def = self.get_model_def(id)
            .ok_or_else(|| format!("Model '{}' not found in catalog", id))?;

        // Do not hold the global registry or catalog locks during loading.
        info!("Loading model '{}' from {:?}...", id, def.path);
        let model = loader(&def)?;
        let arc = Arc::new(model);

        // insert (check again in case another thread loaded it meanwhile)
        let mut map = self.loaded.lock();
        if let Some(existing) = map.get(id) {
            return existing.clone().downcast::<T>().map_err(|_| format!("Model '{}' already has a different type", id));
        }

        map.insert(id.to_string(), arc.clone());
        info!("Model '{}' loaded and registered", id);

        Ok(arc)
    }

    // insert a model directly (for models not in the catalog,
    // or loaded through non-standard means like async init)
    pub fn insert<T: 'static + Send + Sync>(&self, id: &str, model: T) -> Arc<T> {
        let arc = Arc::new(model);
        self.loaded.lock().insert(id.to_string(), arc.clone());
        arc
    }

    pub fn unload(&self, id: &str) -> bool {
        let removed = self.loaded.lock().remove(id).is_some();
        if removed {
            info!("Model '{}' unloaded from registry", id);
        }
        removed
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.loaded.lock().contains_key(id)
    }

    pub fn loaded_ids(&self) -> Vec<String> {
        self.loaded.lock().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, atomic::{AtomicUsize, Ordering}};
    fn def(id: &str) -> ModelDef {
        ModelDef { id: id.into(), name: id.into(), tasks: vec![], description: String::new(), path: id.into() }
    }
    #[test]
    fn concurrent_requests_load_one_shared_model() {
        let registry = Arc::new(ModelRegistry::new());
        registry.set_catalog(vec![def("model")]);
        let calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8).map(|_| {
            let (registry, calls, start) = (registry.clone(), calls.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                registry.get_or_load("model", |_| { calls.fetch_add(1, Ordering::SeqCst); std::thread::sleep(std::time::Duration::from_millis(20)); Ok(42u32) }).unwrap()
            })
        }).collect();
        let models: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(models.iter().all(|m| Arc::ptr_eq(m, &models[0])));
        assert!(registry.get_or_load::<String>("model", |_| panic!("must not replace a different type")).is_err());
        assert_eq!(*registry.get::<u32>("model").unwrap(), 42);
    }
    #[test]
    fn a_loader_can_request_another_model() {
        let registry = ModelRegistry::new();
        registry.set_catalog(vec![def("a"), def("b")]);
        let a = registry.get_or_load("a", |_| Ok(*registry.get_or_load("b", |_| Ok(7u32))? + 1)).unwrap();
        assert_eq!(*a, 8);
        registry.unload("a");
        assert!(registry.get_or_load::<u32>("a", |_| registry.get_or_load::<u32>("a", |_| Ok(1)).map(|v| *v)).is_err());
    }
}
