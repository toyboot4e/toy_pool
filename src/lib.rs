/*!
Pool with reference-counted items
*/

pub mod iter;
pub mod smpsc;

use std::{cmp, marker::PhantomData, ops, slice};

use derivative::Derivative;

use crate::smpsc::{Receiver, Sender};

type Gen = std::num::NonZeroU32;
type GenCounter = u32;

/// Number of existable references = `RefCount::MAX - 1`
pub type RefCount = u16;

/// Newtype of `u32`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug)]
pub struct Handle<T> {
    slot: Slot,
    /// For downgrading to weak handle
    gen: Gen,
    sender: Sender<Message>,
    _ty: PhantomData<fn() -> T>,
}

impl<T> cmp::PartialEq for Handle<T> {
    fn eq(&self, other: &Handle<T>) -> bool {
        // WARNING: it doesn't consider belonging pool
        self.gen == other.gen
    }
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

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        self.sender.send(Message::New(self.slot));

        Self {
            slot: self.slot,
            gen: self.gen,
            sender: self.sender.clone(),
            _ty: Default::default(),
        }
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
pub struct WeakHandle<T> {
    slot: Slot,
    /// For distingushing original item
    gen: Gen,
    _ty: PhantomData<fn() -> T>,
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

#[derive(Debug, Clone)]
pub(crate) struct PoolEntry<T> {
    // TODO: refactor using an enum
    data: Option<T>,
    gen: Gen,
    ref_count: RefCount,
}

/// Dynamic array with reference-counted [`Handle`]s
#[derive(Debug)]
pub struct Pool<T> {
    /// NOTE: we never call [`Vec::remove`]; it aligns (change positions of) other items.
    entries: Vec<PoolEntry<T>>,
    /// Generation counter per [`Pool`]. Another option is per slot.
    gen_count: GenCounter,
    /// Receiver
    rx: Receiver<Message>,
    /// Sender. Cloned and passed to [`Handle`]s
    tx: Sender<Message>,
}

impl<T> Pool<T> {
    pub fn with_capacity(cap: usize) -> Self {
        let (tx, rx) = smpsc::unbounded();
        Self {
            entries: Vec::with_capacity(cap),
            gen_count: 1,
            rx,
            tx,
        }
    }

    /// Update reference counting of internal items and invalidates unreferenced items
    pub fn sync_refcounts_and_invalidate(&mut self) {
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
                        entry.data = None;
                    }
                }
            }
        }
    }

    // /// Force removing a reference-counted node
    // pub unsafe fn remove_node(&mut self, slot: Slot) {
    //     let entry = &mut self.entries[slot.as_usize()];
    //     entry.gen = None;
    // }
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

        let gen = Gen::new(self.gen_count).expect("Generation overflow");

        let entry = PoolEntry {
            data: Some(item),
            gen,
            ref_count: 1, // !
        };

        self.gen_count += 1;

        let slot = match self.find_empty_slot() {
            Some(i) => {
                self.entries[i] = entry;
                i
            }
            None => {
                let i = self.entries.len();
                self.entries.push(entry);
                i
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
    pub fn get(&self, weak_handle: &WeakHandle<T>) -> Option<&T> {
        let entry = &self.entries[weak_handle.slot.to_usize()];
        if entry.gen == weak_handle.gen {
            entry.data.as_ref()
        } else {
            None
        }
    }

    /// Tries to get a mutable reference from a [`WeakHandle`]
    pub fn get_mut(&mut self, weak_handle: &WeakHandle<T>) -> Option<&mut T> {
        let entry = &mut self.entries[weak_handle.slot.to_usize()];
        if entry.gen == weak_handle.gen {
            entry.data.as_mut()
        } else {
            None
        }
    }

    pub fn upgrade(&self, weak_handle: &WeakHandle<T>) -> Option<Handle<T>> {
        let slot = weak_handle.slot.to_usize();
        if slot > self.entries.len() {
            return None;
        }
        let entry = &self.entries[slot];
        if entry.gen == weak_handle.gen {
            Some(Handle {
                slot: weak_handle.slot,
                gen: weak_handle.gen,
                sender: self.tx.clone(),
                _ty: PhantomData,
            })
        } else {
            None
        }
    }
}

impl<T> ops::Index<&Handle<T>> for Pool<T> {
    type Output = T;
    fn index(&self, handle: &Handle<T>) -> &Self::Output {
        self.entries[handle.slot.to_usize()]
            .data
            .as_ref()
            .expect("dropped entry data while there's strong handle!")
    }
}

impl<T> ops::IndexMut<&Handle<T>> for Pool<T> {
    fn index_mut(&mut self, handle: &Handle<T>) -> &mut Self::Output {
        self.entries[handle.slot.to_usize()]
            .data
            .as_mut()
            .expect("dropped entry data while there's strong handle!")
    }
}

impl<T> Pool<T> {
    /// Returns iterator of valid items in this pool
    pub fn iter(&self) -> impl Iterator<Item = &T>
    where
        T: 'static,
    {
        iter::Iter {
            entries: self.entries.iter(),
        }
    }

    /// Returns mutable iterator of valid items in this pool
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T>
    where
        T: 'static,
    {
        iter::IterMut {
            entries: self.entries.iter_mut(),
        }
    }
}

/// # ----- Slot-based accessors -----
impl<T> Pool<T> {
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

    // TODO: use specific iterator?
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.entries.iter().enumerate().filter_map(|(i, entry)| {
            if entry.data.is_some() {
                Some(Slot(i as u32))
            } else {
                None
            }
        })
    }

    /// Iterator of `(Slot, &T)`
    pub fn enumerate_items(&self) -> impl Iterator<Item = (Slot, &T)> {
        self.entries.iter().enumerate().filter_map(|(i, entry)| {
            if let Some(data) = &entry.data {
                Some((Slot(i as u32), data))
            } else {
                None
            }
        })
    }

    /// Iterator of `(Slot, &mut T)`
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
