//! MIDI learn: CC assignments and the values they set.
//!
//! nice-plug only lets the editor tell the host about parameter changes. So a learned CC changes
//! the sound right away through an override the audio thread applies on top of the parameter
//! value, and the editor, while it's open, forwards the value to the host. Once the host's value
//! matches, or the host changes the parameter itself, the override is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use atomic_float::AtomicF32;
use nice_plug::prelude::*;

/// A learned value the host hasn't caught up with yet. Audio thread only.
#[derive(Debug, Clone, Copy)]
pub struct Override {
    pub ptr: ParamPtr,
    /// Normalized value set by the CC
    pub value: f32,
    /// The parameter's normalized value when the override was set
    pub base: f32,
}

/// Parameter reads with overrides applied.
pub struct Overrides<'a>(pub &'a [Override]);

impl Overrides<'_> {
    #[inline]
    fn find(&self, ptr: ParamPtr) -> Option<f32> {
        self.0.iter().find(|o| o.ptr == ptr).map(|o| o.value)
    }

    #[inline]
    pub fn float(&self, p: &FloatParam) -> f32 {
        match self.find(p.as_ptr()) {
            Some(n) => p.preview_plain(n),
            None => p.value(),
        }
    }

    #[inline]
    pub fn int(&self, p: &IntParam) -> i32 {
        match self.find(p.as_ptr()) {
            Some(n) => p.preview_plain(n),
            None => p.value(),
        }
    }

    #[inline]
    pub fn bool(&self, p: &BoolParam) -> bool {
        match self.find(p.as_ptr()) {
            Some(n) => p.preview_plain(n),
            None => p.value(),
        }
    }

    #[inline]
    pub fn enumv<T: Enum + PartialEq + 'static>(&self, p: &EnumParam<T>) -> T {
        match self.find(p.as_ptr()) {
            Some(n) => p.preview_plain(n),
            None => p.value(),
        }
    }
}

const NONE: u32 = 0;

pub struct MidiLearn {
    /// All parameters, in a fixed order. Indices below refer to this list.
    params: Vec<(String, ParamPtr)>,
    index_by_ptr: HashMap<ParamPtr, usize>,
    /// CC number → parameter index + 1, `0` = unassigned
    cc_map: [AtomicU32; 128],
    /// Parameter index + 1 waiting for a CC, `0` = not learning
    learning: AtomicU32,
    /// Set when `cc_map` changed and needs to be persisted
    pub map_changed: AtomicBool,
    /// Latest CC value per parameter for the editor to forward to the host, NaN = nothing new
    pending: Vec<AtomicF32>,
    pub has_pending: AtomicBool,
}

impl MidiLearn {
    pub fn new(params: &impl Params) -> Self {
        let params: Vec<(String, ParamPtr)> = params
            .param_map()
            .into_iter()
            .map(|(id, ptr, _)| (id, ptr))
            .collect();
        let index_by_ptr = params
            .iter()
            .enumerate()
            .map(|(i, (_, ptr))| (*ptr, i))
            .collect();
        let pending = params.iter().map(|_| AtomicF32::new(f32::NAN)).collect();

        Self {
            params,
            index_by_ptr,
            cc_map: std::array::from_fn(|_| AtomicU32::new(NONE)),
            learning: AtomicU32::new(NONE),
            map_changed: AtomicBool::new(false),
            pending,
            has_pending: AtomicBool::new(false),
        }
    }

    fn index_of(&self, ptr: ParamPtr) -> Option<usize> {
        self.index_by_ptr.get(&ptr).copied()
    }

    /// Starts learning for `ptr`: the next CC gets assigned to it.
    pub fn start_learning(&self, ptr: ParamPtr) {
        if let Some(i) = self.index_of(ptr) {
            self.learning.store(i as u32 + 1, Ordering::Release);
        }
    }

    pub fn cancel_learning(&self) {
        self.learning.store(NONE, Ordering::Release);
    }

    pub fn is_learning(&self, ptr: ParamPtr) -> bool {
        let l = self.learning.load(Ordering::Acquire);
        l != NONE && self.index_of(ptr) == Some(l as usize - 1)
    }

    /// The CC assigned to `ptr`, if any.
    pub fn cc_for(&self, ptr: ParamPtr) -> Option<u8> {
        let target = self.index_of(ptr)? as u32 + 1;
        (0..128u8).find(|cc| self.cc_map[*cc as usize].load(Ordering::Relaxed) == target)
    }

    pub fn forget(&self, ptr: ParamPtr) {
        if let Some(cc) = self.cc_for(ptr) {
            self.cc_map[cc as usize].store(NONE, Ordering::Release);
            self.map_changed.store(true, Ordering::Release);
        }
    }

    /// Current assignments as `(cc, parameter id)`, for persisting.
    pub fn assignments(&self) -> Vec<(u8, String)> {
        (0..128u8)
            .filter_map(
                |cc| match self.cc_map[cc as usize].load(Ordering::Acquire) {
                    NONE => None,
                    i => Some((cc, self.params[i as usize - 1].0.clone())),
                },
            )
            .collect()
    }

    /// Replaces all assignments, e.g. from the saved state. Unknown parameter IDs are ignored.
    pub fn set_assignments(&self, map: &[(u8, String)]) {
        for slot in &self.cc_map {
            slot.store(NONE, Ordering::Relaxed);
        }
        for (cc, id) in map {
            if let Some(i) = self.params.iter().position(|(pid, _)| pid == id) {
                if let Some(slot) = self.cc_map.get(*cc as usize) {
                    slot.store(i as u32 + 1, Ordering::Release);
                }
            }
        }
    }

    /// Audio thread: handles a CC. Returns the parameter and normalized value it sets, if any.
    pub fn handle_cc(&self, cc: u8, value: f32) -> Option<(ParamPtr, f32)> {
        let slot = self.cc_map.get(cc as usize)?;
        let learning = self.learning.swap(NONE, Ordering::AcqRel);
        if learning != NONE {
            // A CC can only control one parameter
            for other in &self.cc_map {
                let _ = other.compare_exchange(learning, NONE, Ordering::AcqRel, Ordering::Relaxed);
            }
            slot.store(learning, Ordering::Release);
            self.map_changed.store(true, Ordering::Release);
        }

        let index = match slot.load(Ordering::Acquire) {
            NONE => return None,
            i => i as usize - 1,
        };
        self.pending[index].store(value, Ordering::Release);
        self.has_pending.store(true, Ordering::Release);
        Some((self.params[index].1, value))
    }

    /// Editor: takes the values to forward to the host.
    pub fn take_pending(&self, mut f: impl FnMut(ParamPtr, f32)) {
        if !self.has_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        for (i, pending) in self.pending.iter().enumerate() {
            let v = pending.swap(f32::NAN, Ordering::AcqRel);
            if !v.is_nan() {
                f(self.params[i].1, v);
            }
        }
    }
}

/// Audio thread: sets or updates the override for `ptr`. Never allocates as long as `overrides`
/// has spare capacity; drops the value otherwise.
pub fn set_override(overrides: &mut Vec<Override>, ptr: ParamPtr, value: f32) {
    if let Some(o) = overrides.iter_mut().find(|o| o.ptr == ptr) {
        o.value = value;
    } else if overrides.len() < overrides.capacity() {
        // SAFETY: the pointer comes from the plugin's own parameter map
        let base = unsafe { ptr.unmodulated_normalized_value() };
        overrides.push(Override { ptr, value, base });
    }
}

/// Audio thread: drops overrides the host has caught up with or overwritten.
pub fn prune_overrides(overrides: &mut Vec<Override>) {
    overrides.retain(|o| {
        // SAFETY: see above
        let current = unsafe { o.ptr.unmodulated_normalized_value() };
        (current - o.value).abs() > 1e-4 && (current - o.base).abs() <= 1e-4
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::KlopferParams;

    #[test]
    fn learns_cc_and_persists_assignments() {
        let params = KlopferParams::default();
        let learn = MidiLearn::new(&params);
        let cutoff = params.pads[2].cutoff.as_ptr();

        learn.start_learning(cutoff);
        assert!(learn.is_learning(cutoff));
        let (ptr, value) = learn.handle_cc(74, 0.5).unwrap();
        assert_eq!(ptr, cutoff);
        assert_eq!(value, 0.5);
        assert!(!learn.is_learning(cutoff));
        assert_eq!(learn.cc_for(cutoff), Some(74));

        let map = learn.assignments();
        assert_eq!(map, vec![(74, String::from("cutoff_3"))]);

        let other = MidiLearn::new(&params);
        other.set_assignments(&map);
        assert_eq!(other.cc_for(cutoff), Some(74));

        learn.forget(cutoff);
        assert!(learn.handle_cc(74, 1.0).is_none());
    }

    #[test]
    fn relearning_moves_the_parameter() {
        let params = KlopferParams::default();
        let learn = MidiLearn::new(&params);
        let vol = params.pads[0].volume.as_ptr();
        learn.start_learning(vol);
        learn.handle_cc(1, 0.0);
        learn.start_learning(vol);
        learn.handle_cc(2, 0.0);
        assert_eq!(learn.cc_for(vol), Some(2));
        assert!(learn.handle_cc(1, 0.3).is_none());
    }

    #[test]
    fn pending_values_reach_the_editor_once() {
        let params = KlopferParams::default();
        let learn = MidiLearn::new(&params);
        let pan = params.pads[0].pan.as_ptr();
        learn.start_learning(pan);
        learn.handle_cc(10, 0.25);

        let mut got = vec![];
        learn.take_pending(|p, v| got.push((p, v)));
        assert_eq!(got, vec![(pan, 0.25)]);
        got.clear();
        learn.take_pending(|p, v| got.push((p, v)));
        assert!(got.is_empty());
    }

    #[test]
    fn overrides_apply_until_the_host_catches_up() {
        let params = KlopferParams::default();
        let reso = &params.pads[0].resonance;
        let mut overrides = Vec::with_capacity(4);
        set_override(&mut overrides, reso.as_ptr(), 0.8);
        assert!((Overrides(&overrides).float(reso) - 0.8).abs() < 1e-6);

        // Host still at the old value: override stays
        prune_overrides(&mut overrides);
        assert_eq!(overrides.len(), 1);
    }
}
