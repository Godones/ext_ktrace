use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};

use lock_api::{Mutex, RawMutex};

use crate::{KprobeAuxiliaryOps, KprobeOps, ProbePoint, UniProbe};

/// A manager for kprobes.
#[derive(Debug)]
pub struct ProbeManager<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    break_list: Mutex<L, BTreeMap<usize, Vec<UniProbe<L, F>>>>,
    debug_list: Mutex<L, BTreeMap<usize, Vec<UniProbe<L, F>>>>,
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> Default for ProbeManager<L, F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: RawMutex + 'static, F: KprobeAuxiliaryOps> ProbeManager<L, F> {
    /// Create a new kprobe manager.
    pub const fn new() -> Self {
        ProbeManager {
            break_list: Mutex::new(BTreeMap::new()),
            debug_list: Mutex::new(BTreeMap::new()),
        }
    }
    /// Insert a kprobe into the manager.
    pub fn insert_probe(&self, probe: UniProbe<L, F>) {
        let probe_point = probe.probe_point().clone();
        self.insert_break_point(probe_point.break_address(), probe.clone());
        self.insert_debug_point(probe_point.debug_address(), probe);
    }

    /// Insert a kprobe into the break_list.
    ///
    /// # Parameters
    /// - `address`: The address of the kprobe, obtained from `KprobePoint::break_address()` or `KprobeBuilder::probe_addr()`.
    /// - `kprobe`: The instance of the kprobe.
    fn insert_break_point(&self, address: usize, probe: UniProbe<L, F>) {
        let mut list = self.break_list.lock();
        let list = list.entry(address).or_default();
        list.push(probe);
    }

    /// Insert a kprobe into the debug_list.
    ///
    /// # Parameters
    /// - `address`: The address of the kprobe, obtained from `KprobePoint::debug_address()`.
    /// - `kprobe`: The instance of the kprobe.
    ///
    fn insert_debug_point(&self, address: usize, probe: UniProbe<L, F>) {
        let mut list = self.debug_list.lock();
        let list = list.entry(address).or_default();
        list.push(probe);
    }

    /// Get the list of kprobes registered at a breakpoint address.
    pub fn get_break_list(&self, address: usize) -> Option<Vec<UniProbe<L, F>>> {
        self.break_list.lock().get(&address).cloned()
    }

    /// Get the list of kprobes registered at a debug address.
    pub fn get_debug_list(&self, address: usize) -> Option<Vec<UniProbe<L, F>>> {
        self.debug_list.lock().get(&address).cloned()
    }

    /// Get the number of kprobes registered at a breakpoint address.
    pub fn kprobe_num(&self, address: usize) -> usize {
        self.break_list_len(address)
    }

    pub(crate) fn replace_debug_list_with_new_address(
        &self,
        old_address: usize,
        new_address: usize,
    ) {
        let mut debug_list = self.debug_list.lock();
        if let Some(list) = debug_list.remove(&old_address) {
            debug_list.insert(new_address, list);
        }
    }

    #[inline]
    fn break_list_len(&self, address: usize) -> usize {
        self.break_list
            .lock()
            .get(&address)
            .map(|list| list.len())
            .unwrap_or(0)
    }

    /// Remove a kprobe from the manager.
    pub fn remove_probe(&self, probe: UniProbe<L, F>) {
        let probe_point = probe.probe_point();
        self.remove_one_break(probe_point.break_address(), &probe);
        self.remove_one_debug(probe_point.debug_address(), &probe);
    }

    /// Remove a kprobe from the break_list.
    fn remove_one_break(&self, address: usize, probe: &UniProbe<L, F>) {
        let mut list = self.break_list.lock();
        if let Some(list) = list.get_mut(&address) {
            list.retain(|x| x != probe);
        }
        let len = list.get(&address).map(|list| list.len()).unwrap_or(0);
        if len == 0 {
            list.remove(&address);
        }
    }

    /// Remove a kprobe from the debug_list.
    fn remove_one_debug(&self, address: usize, probe: &UniProbe<L, F>) {
        let mut list = self.debug_list.lock();
        if let Some(list) = list.get_mut(&address) {
            list.retain(|x| x != probe);
        }
        let len = list.get(&address).map(|list| list.len()).unwrap_or(0);
        if len == 0 {
            list.remove(&address);
        }
    }
}

/// A list of kprobe points.
pub type ProbePointList<F> = BTreeMap<usize, Arc<ProbePoint<F>>>;
