/*!
Pool with reference-counted items

Items in the [`Pool`] will be reference-counted with strong [`Handle`]s. When no [`Handle`] is
referring to an item, it can be removed on synchronization, or you can handle it manually.

Note that the pool does NOT drop unreferenced items until it's synced. Also it's single-thread only,
for no particular reason.
*/

// TODO: add length tracking and implement FuseIterator for iterator types

pub mod iter;
pub mod smpsc;
pub mod tree;

#[cfg(test)]
mod test;

#[cfg(feature = "igri")]
use igri::Inspect;

use std::{cmp, marker::PhantomData, ops, slice};

use derivative::Derivative;

use crate::smpsc::{Receiver, Sender};

type Gen = std::num::NonZeroU32;

/// Type for reference counting
pub type RefCount = u16;

/// Newtype of `u32`
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "igri", derive(Inspect))]
pub struct Slot(u32);

impl Slot {
    pub fn to_usize(&self) -> usize {
        self.0 as usize
    }
}

/// Reference counting message (New | Drop)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Message {
    New(Slot),
    Drop(Slot),
}

/// Owing index to an item in a [`Pool`]
#[derive(Derivative)]
#[derivative(Debug, PartialEq, Clone)]
#[cfg_attr(
    feature = "igri",
    derive(Inspect),
    inspect(with = "inspect_handle", bounds = "")
)]
pub struct Handle<T> {
    slot: Slot,
    /// For downgrading to weak handle
    gen: Gen,
    #[derivative(PartialEq = "ignore")]
    sender: Sender<Message>,
    _ty: PhantomData<fn() -> T>,
}

#[cfg(feature = "igri")]
fn inspect_handle<'a, T>(handle: &mut Handle<T>, ui: &igri::imgui::Ui, label: &str) {
    igri::Inspect::inspect(&mut handle.slot.0, ui, label);
}

impl<T> Handle<T> {
    /// Index that corrresponds to memory location
    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn downgrade(self) -> WeakHandle<T> {
        WeakHandle {
            slot: self.slot,
            gen: self.gen,
            _ty: PhantomData,
        }
    }

    pub fn to_downgraded(&self) -> WeakHandle<T> {
        self.clone().downgrade()
    }
}

impl<T> Drop for Handle<T> {
    fn drop(&mut self) {
        self.sender.send(Message::Drop(self.slot));
    }
}

/// Non-owing index to an item in a [`Pool`]
///
/// The item is identified with generational index.
#[derive(Derivative)]
#[derivative(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(
    feature = "igri",
    derive(Inspect),
    inspect(with = "inspect_weak_handle", bounds = "")
)]
pub struct WeakHandle<T> {
    slot: Slot,
    /// For distingushing original item
    gen: Gen,
    _ty: PhantomData<fn() -> T>,
}

#[cfg(feature = "igri")]
fn inspect_weak_handle<'a, T>(handle: &mut WeakHandle<T>, ui: &igri::imgui::Ui, label: &str) {
    igri::Inspect::inspect(&mut handle.slot.0, ui, label);
}

impl<T> WeakHandle<T> {
    /// Index that corrresponds to memory location
    pub fn slot(&self) -> Slot {
        self.slot
    }
}

impl<T> From<Handle<T>> for WeakHandle<T> {
    fn from(h: Handle<T>) -> Self {
        Self {
            slot: h.slot,
            gen: h.gen,
            _ty: PhantomData,
        }
    }
}

// TODO: make it smaller
#[derive(Debug, Clone)]
pub(crate) struct PoolEntry<T> {
    // TODO: refactor using an enum
    data: Option<T>,
    gen: Gen,
    ref_count: RefCount,
}

/// Dynamic array with reference-counted [`Handle`]s
///
/// Be sure to call message syncing method to track reference counts.
#[derive(Debug)]
#[cfg_attr(
    feature = "igri",
    derive(Inspect),
    inspect(with = "inspect_pool", bounds = "T: Inspect")
)]
pub struct Pool<T> {
    /// NOTE: we never call [`Vec::remove`]; it aligns (change positions of) other items.
    entries: Vec<PoolEntry<T>>,
    /// Receiver
    #[cfg_attr(feature = "igri", inspect(skip))]
    rx: Receiver<Message>,
    /// Sender. Cloned and passed to [`Handle`]s
    #[cfg_attr(feature = "igri", inspect(skip))]
    tx: Sender<Message>,
}

#[cfg(feature = "igri")]
fn inspect_pool<'a, T>(pool: &'a mut Pool<T>, ui: &igri::imgui::Ui, label: &str)
where
    T: igri::Inspect,
{
    igri::seq(pool.entries.iter_mut().map(|e| &mut e.data), ui, label);
}

impl<T> Pool<T> {
    pub fn with_capacity(cap: usize) -> Self {
        let (tx, rx) = smpsc::unbounded();
        Self {
            entries: Vec::with_capacity(cap),
            rx,
            tx,
        }
    }
}

/// # ----- Reference counter synchronization --
impl<T> Pool<T> {
    /// Update reference counts letting user visit item with zero reference counts.
    pub fn sync_refcounts(&mut self, mut on_zero: impl FnMut(&mut Self, Slot)) {
        while let Some(mes) = self.rx.recv() {
            match mes {
                Message::New(slot) => {
                    let e = &mut self.entries[slot.to_usize()];
                    e.ref_count += 1;
                }
                Message::Drop(slot) => {
                    let entry = &mut self.entries[slot.to_usize()];
                    entry.ref_count -= 1;
                    if entry.ref_count == 0 {
                        on_zero(self, slot);
                    }
                }
            }
        }
    }

    /// Updates reference counts and invalidates unreferenced items
    pub fn sync_refcounts_and_invalidate(&mut self) {
        self.sync_refcounts(|p, slot| {
            p.invalidate_unreferenced(slot);
        })
    }

    /// Invalidates an entry with zero reference count manually
    pub fn invalidate_unreferenced(&mut self, slot: Slot) -> bool {
        let e = &mut self.entries[slot.to_usize()];
        assert!(e.ref_count == 0);
        if e.data.is_none() {
            return false;
        }
        e.data = None;
        true
    }
}

/// # ----- Handle-based accessors -----
impl<T> Pool<T> {
    /// TODO: Consider tracking empty slot
    fn find_empty_slot(&mut self) -> Option<usize> {
        for i in 0..self.entries.len() {
            if let Some(entry) = self.entries.get(i) {
                if entry.data.is_none() {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Inserts the item and returns a strong [`Handle`] for it
    pub fn add(&mut self, item: impl Into<T>) -> Handle<T> {
        let item = item.into();

        let (gen, slot) = match self.find_empty_slot() {
            Some(i) => {
                let entry = &mut self.entries[i];
                entry.data = Some(item);
                entry.gen = Gen::new(entry.gen.get() + 1).expect("Generation overflow!");
                (entry.gen, i)
            }
            None => {
                let gen = unsafe { Gen::new_unchecked(1) };
                let entry = PoolEntry {
                    data: Some(item),
                    gen,
                    // count the initial handle below
                    ref_count: 1,
                };

                let i = self.entries.len();
                self.entries.push(entry);
                (gen, i)
            }
        };

        Handle {
            slot: Slot(slot as u32),
            gen,
            sender: self.tx.clone(),
            _ty: Default::default(),
        }
    }

    /// Tries to get a reference from a [`WeakHandle`]
    pub fn get(&self, weak: &WeakHandle<T>) -> Option<&T> {
        let entry = &self.entries[weak.slot.to_usize()];
        if entry.gen == weak.gen {
            entry.data.as_ref()
        } else {
            None
        }
    }

    /// Tries to get a mutable reference from a [`WeakHandle`]
    pub fn get_mut(&mut self, weak: &WeakHandle<T>) -> Option<&mut T> {
        let entry = &mut self.entries[weak.slot.to_usize()];
        if entry.gen == weak.gen {
            entry.data.as_mut()
        } else {
            None
        }
    }
}

impl<T> ops::Index<&Handle<T>> for Pool<T> {
    type Output = T;
    fn index(&self, handle: &Handle<T>) -> &Self::Output {
        let entry = &self.entries[handle.slot.to_usize()];
        debug_assert!(entry.ref_count > 0);
        entry
            .data
            .as_ref()
            .expect("dropped entry found while there's strong at least one handle!")
    }
}

impl<T> ops::IndexMut<&Handle<T>> for Pool<T> {
    fn index_mut(&mut self, handle: &Handle<T>) -> &mut Self::Output {
        let entry = &mut self.entries[handle.slot.to_usize()];
        debug_assert!(entry.ref_count > 0);
        entry
            .data
            .as_mut()
            .expect("dropped entry found while there's strong at least one handle!")
    }
}

/// # ----- Slot-based accessors -----
impl<T> Pool<T> {
    /// Tries to upgrade the weak handle to a strong handle. Fails if it's already removed or IF THE
    /// REF COUNT IS ALREADY ZERO. This is for protecting [`Pool::sync_refcounts`], but this design
    /// may change.
    pub fn upgrade(&self, weak: &WeakHandle<T>) -> Option<Handle<T>> {
        let slot = weak.slot.to_usize();
        if slot > self.entries.len() {
            return None;
        }

        let entry = &self.entries[slot];
        if entry.ref_count == 0 {
            return None;
        }

        if entry.gen == weak.gen {
            Some(Handle {
                slot: weak.slot,
                gen: weak.gen,
                sender: self.tx.clone(),
                _ty: PhantomData,
            })
        } else {
            None
        }
    }

    /// Retruns the item if it's valid
    pub fn get_by_slot(&self, slot: Slot) -> Option<&T> {
        let entry = self.entries.get(slot.to_usize())?;
        entry.data.as_ref()
    }

    /// Retruns the item if it's valid
    pub fn get_mut_by_slot(&mut self, slot: Slot) -> Option<&mut T> {
        let entry = self.entries.get_mut(slot.to_usize())?;
        entry.data.as_mut()
    }

    /// Retruns the item if it's valid
    pub fn get2_mut_by_slot(&mut self, s0: Slot, s1: Slot) -> Option<(&mut T, &mut T)> {
        assert_ne!(s0, s1);

        let e0 = self
            .entries
            .get_mut(s0.to_usize())
            .map(|e| e as *mut PoolEntry<T>)?;
        let e1 = self.entries.get_mut(s1.to_usize())?;

        let d0 = unsafe { &mut *e0 }.data.as_mut()?;
        let d1 = e1.data.as_mut()?;

        Some((d0, d1))
    }

    /// Returns slots of existing items. NOTE: It contains unreferenced items as long as they're not
    /// yet removed.
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.entries.iter().enumerate().filter_map(|(i, entry)| {
            if entry.data.is_some() {
                Some(Slot(i as u32))
            } else {
                None
            }
        })
    }
}

/// # ----- Iterators -----
impl<T> Pool<T> {
    /// Returns an iterator of valid items in this pool
    pub fn iter(&self) -> impl Iterator<Item = &T>
    where
        T: 'static,
    {
        iter::Iter {
            entries: self.entries.iter(),
        }
    }

    /// Returns an mutable iterator of valid items in this pool
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T>
    where
        T: 'static,
    {
        iter::IterMut {
            entries: self.entries.iter_mut(),
        }
    }

    /// Returns an iterator of `(Slot, &T)`
    pub fn enumerate_items(&self) -> impl Iterator<Item = (Slot, &T)> {
        self.entries.iter().enumerate().filter_map(|(i, entry)| {
            if let Some(data) = &entry.data {
                Some((Slot(i as u32), data))
            } else {
                None
            }
        })
    }

    /// Returns an iterator of `(Slot, &mut T)`
    pub fn enumerate_items_mut(&mut self) -> impl Iterator<Item = (Slot, &mut T)> {
        self.entries
            .iter_mut()
            .enumerate()
            .filter_map(|(i, entry)| {
                if let Some(data) = &mut entry.data {
                    Some((Slot(i as u32), data))
                } else {
                    None
                }
            })
    }
}
