use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};

use lock_api::RawMutex;

use crate::{KprobeAuxiliaryOps, KprobeOps, ProbePoint, UniProbe};

/// A manager for kprobes.
#[derive(Debug)]
pub struct ProbeManager<L: RawMutex + 'static, F: KprobeAuxiliaryOps> {
    break_list: BTreeMap<usize, Vec<UniProbe<L, F>>>,
    debug_list: BTreeMap<usize, Vec<UniProbe<L, F>>>,
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
            break_list: BTreeMap::new(),
            debug_list: BTreeMap::new(),
        }
    }
    /// Insert a kprobe into the manager.
    pub fn insert_probe(&mut self, probe: UniProbe<L, F>) {
        let probe_point = probe.probe_point().clone();
        self.insert_break_point(probe_point.break_address(), probe.clone());
        self.insert_debug_point(probe_point.debug_address(), probe);
    }

    /// Insert a kprobe into the break_list.
    ///
    /// # Parameters
    /// - `address`: The address of the kprobe, obtained from `KprobePoint::break_address()` or `KprobeBuilder::probe_addr()`.
    /// - `kprobe`: The instance of the kprobe.
    fn insert_break_point(&mut self, address: usize, probe: UniProbe<L, F>) {
        let list = self.break_list.entry(address).or_default();
        list.push(probe);
    }

    /// Insert a kprobe into the debug_list.
    ///
    /// # Parameters
    /// - `address`: The address of the kprobe, obtained from `KprobePoint::debug_address()`.
    /// - `kprobe`: The instance of the kprobe.
    ///
    fn insert_debug_point(&mut self, address: usize, probe: UniProbe<L, F>) {
        let list = self.debug_list.entry(address).or_default();
        list.push(probe);
    }

    /// Get the list of kprobes registered at a breakpoint address.
    pub fn get_break_list(&self, address: usize) -> Option<&Vec<UniProbe<L, F>>> {
        self.break_list.get(&address)
    }

    /// Get the list of kprobes registered at a debug address.
    pub fn get_debug_list(&self, address: usize) -> Option<&Vec<UniProbe<L, F>>> {
        self.debug_list.get(&address)
    }

    /// Get the number of kprobes registered at a breakpoint address.
    pub fn kprobe_num(&self, address: usize) -> usize {
        self.break_list_len(address)
    }

    pub(crate) fn replace_debug_list_with_new_address(
        &mut self,
        old_address: usize,
        new_address: usize,
    ) {
        if let Some(list) = self.debug_list.remove(&old_address) {
            self.debug_list.insert(new_address, list);
        }
    }

    #[inline]
    fn break_list_len(&self, address: usize) -> usize {
        self.break_list
            .get(&address)
            .map(|list| list.len())
            .unwrap_or(0)
    }
    #[inline]
    fn debug_list_len(&self, address: usize) -> usize {
        self.debug_list
            .get(&address)
            .map(|list| list.len())
            .unwrap_or(0)
    }

    /// Remove a kprobe from the manager.
    pub fn remove_probe(&mut self, probe: UniProbe<L, F>) {
        let probe_point = probe.probe_point().clone();
        self.remove_one_break(probe_point.break_address(), &probe);
        self.remove_one_debug(probe_point.debug_address(), &probe);
    }

    /// Remove a kprobe from the break_list.
    fn remove_one_break(&mut self, address: usize, probe: &UniProbe<L, F>) {
        if let Some(list) = self.break_list.get_mut(&address) {
            list.retain(|x| x != probe);
        }
        if self.break_list_len(address) == 0 {
            self.break_list.remove(&address);
        }
    }

    /// Remove a kprobe from the debug_list.
    fn remove_one_debug(&mut self, address: usize, probe: &UniProbe<L, F>) {
        if let Some(list) = self.debug_list.get_mut(&address) {
            list.retain(|x| x != probe);
        }
        if self.debug_list_len(address) == 0 {
            self.debug_list.remove(&address);
        }
    }
}

/// A list of kprobe points.
pub type ProbePointList<F> = BTreeMap<usize, Arc<ProbePoint<F>>>;
