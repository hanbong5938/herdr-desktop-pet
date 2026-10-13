//! Profile-bound manager admission. No host I/O runs in a domain transaction.
use crate::{Registry, Shared};
use desktop_pet_contracts::ProfileStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

static PROFILES: LazyLock<Mutex<Vec<Weak<Shared>>>> = LazyLock::new(|| Mutex::new(Vec::new()));
pub(crate) struct PendingMutation {
    epoch: u64,
    cancelled: Arc<AtomicBool>,
    offered: bool,
}
impl PendingMutation {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
pub(crate) fn register(shared: &Arc<Shared>) -> Result<(), String> {
    let mut profiles = PROFILES
        .lock()
        .map_err(|_| "manager profile registry is poisoned")?;
    profiles.retain(|profile| profile.strong_count() != 0);
    if !profiles
        .iter()
        .any(|profile| profile.ptr_eq(&Arc::downgrade(shared)))
    {
        profiles.push(Arc::downgrade(shared));
    }
    Ok(())
}
fn shared_for(profile: &ProfileStore) -> Result<Arc<Shared>, String> {
    let profiles = PROFILES
        .lock()
        .map_err(|_| "manager profile registry is poisoned")?;
    for candidate in profiles.iter().filter_map(Weak::upgrade) {
        let store = candidate
            .store
            .lock()
            .map_err(|_| "Herdr profile is poisoned")?;
        let matches = store.as_ref().is_some_and(|store| {
            store.config_dir() == profile.config_dir()
                && Arc::ptr_eq(store.lifecycle_lock(), profile.lifecycle_lock())
        });
        drop(store);
        if matches {
            return Ok(candidate);
        }
    }
    Err("manager profile has no configured Herdr owner".to_owned())
}
fn registry_allowed(shared: &Shared, registry: &Registry) -> Result<(), String> {
    if shared.stop.load(Ordering::Acquire) || !registry.enabled || registry.finalized {
        Err("Herdr manager authority is retired".to_owned())
    } else {
        Ok(())
    }
}
pub(crate) fn mutation_allowed(profile: &ProfileStore) -> Result<(), String> {
    profile.verify_owner()?;
    let shared = shared_for(profile)?;
    shared
        .runtime
        .get()
        .ok_or("Herdr integration is not started")?
        .mutation_admission()?;
    let mut admitted = Err("Herdr manager authority is unavailable".to_owned());
    shared.transaction(&mut |domain, registry| {
        admitted = domain
            .mutation_allowed()
            .and_then(|_| registry_allowed(&shared, registry));
    })?;
    admitted
}
pub(crate) struct ManagerMutationFence {
    shared: Arc<Shared>,
    operation: u64,
    epoch: u64,
    cancelled: Arc<AtomicBool>,
}
pub(crate) fn mutation_fence(profile: &ProfileStore) -> Result<ManagerMutationFence, String> {
    profile.verify_owner()?;
    let shared = shared_for(profile)?;
    shared
        .runtime
        .get()
        .ok_or("Herdr integration is not started")?
        .mutation_admission()?;
    let operation = crate::NEXT_OPERATION.fetch_add(1, Ordering::Relaxed);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut admitted = Err("Herdr manager authority is unavailable".to_owned());
    shared.transaction(&mut |domain, registry| {
        admitted = (|| {
            domain.mutation_allowed()?;
            registry_allowed(&shared, registry)?;
            if registry.manager_operations.len() >= crate::MAX_OPERATIONS {
                return Err("too many pending manager operations".to_owned());
            }
            let epoch = registry.epoch;
            registry.manager_operations.insert(
                operation,
                PendingMutation {
                    epoch,
                    cancelled: Arc::clone(&cancelled),
                    offered: false,
                },
            );
            Ok(epoch)
        })();
    })?;
    Ok(ManagerMutationFence {
        shared,
        operation,
        epoch: admitted?,
        cancelled,
    })
}
impl ManagerMutationFence {
    /// Linearizes handoff with Prepare and retire. Once offered, closing or
    /// retiring never kills/replays the CLI; its real backend journal owns the
    /// result. A denied offer leaves published evidence for Unknown review.
    pub(crate) fn offer(&self) -> Result<(), String> {
        self.shared
            .runtime
            .get()
            .ok_or("Herdr integration is not started")?
            .mutation_admission()?;
        let mut admitted = Err("manager operation was retired before handoff".to_owned());
        self.shared.transaction(&mut |domain, registry| {
            admitted = (|| {
                domain.mutation_allowed()?;
                registry_allowed(&self.shared, registry)?;
                if registry.epoch != self.epoch || self.cancelled.load(Ordering::Acquire) {
                    return Err("manager operation was retired before handoff".to_owned());
                }
                let operation = registry
                    .manager_operations
                    .get_mut(&self.operation)
                    .ok_or("manager operation is no longer registered")?;
                if operation.epoch != self.epoch || operation.offered {
                    return Err("manager operation cannot be offered again".to_owned());
                }
                operation.offered = true;
                Ok(())
            })();
        })?;
        admitted
    }
}
impl Drop for ManagerMutationFence {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.shared.registry.lock() {
            registry.manager_operations.remove(&self.operation);
        }
    }
}
