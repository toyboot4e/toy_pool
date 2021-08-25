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
type RefCount = u16;

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
    item: T,
    /// None if this entry is invalid
    gen: Option<Gen>,
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

    /// Update reference counting of internal items and remove unreferenced nodes
    pub fn sync_refcounts(&mut self) {
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
                        entry.gen = None;
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
    fn find_empty_slot(&mut self) -> Option<usize> {
        for i in 0..self.entries.len() {
            if let Some(e) = self.entries.get(i) {
                if e.gen.is_none() {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Returns a reference-counted [`Handle`] for the given item
    pub fn add(&mut self, item: impl Into<T>) -> Handle<T> {
        let item = item.into();

        let gen = Gen::new(self.gen_count);

        let entry = PoolEntry {
            item,
            gen: gen.clone(),
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
            gen: gen.unwrap(),
            sender: self.tx.clone(),
            _ty: Default::default(),
        }
    }

    /// Tries to get a reference from a [`WeakHandle`]
    ///
    /// For strong [`Handle`]s, use index (`pool[handle]`).
    pub fn get(&self, weak_handle: &WeakHandle<T>) -> Option<&T> {
        let entry = &self.entries[weak_handle.slot.to_usize()];
        if entry.gen == Some(weak_handle.gen) {
            Some(&entry.item)
        } else {
            None
        }
    }

    /// Tries to get a mutable reference from a [`WeakHandle`]
    ///
    /// For strong [`Handle`]s, use index (`pool[handle]`).
    pub fn get_mut(&mut self, weak_handle: &WeakHandle<T>) -> Option<&mut T> {
        let entry = &mut self.entries[weak_handle.slot.to_usize()];
        if entry.gen == Some(weak_handle.gen) {
            Some(&mut entry.item)
        } else {
            None
        }
    }
}

impl<T> ops::Index<&Handle<T>> for Pool<T> {
    type Output = T;
    fn index(&self, handle: &Handle<T>) -> &Self::Output {
        &self.entries[handle.slot.to_usize()].item
    }
}

impl<T> ops::IndexMut<&Handle<T>> for Pool<T> {
    fn index_mut(&mut self, handle: &Handle<T>) -> &mut Self::Output {
        &mut self.entries[handle.slot.to_usize()].item
    }
}

impl<T> ops::Index<&WeakHandle<T>> for Pool<T> {
    type Output = T;
    fn index(&self, handle: &WeakHandle<T>) -> &Self::Output {
        let entry = &self.entries[handle.slot.to_usize()];
        assert!(entry.gen == Some(handle.gen));
        &entry.item
    }
}

impl<T> ops::IndexMut<&WeakHandle<T>> for Pool<T> {
    fn index_mut(&mut self, handle: &WeakHandle<T>) -> &mut Self::Output {
        let entry = &mut self.entries[handle.slot.to_usize()];
        assert!(entry.gen == Some(handle.gen));
        &mut entry.item
    }
}

impl<T> Pool<T> {
    /// Iterator of valid items in this pool
    pub fn iter(&self) -> impl Iterator<Item = &T>
    where
        T: 'static,
    {
        iter::Iter {
            entries: self.entries.iter(),
        }
    }

    /// Mutable iterator of valid items in this pool
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
        entry.gen.and(Some(&entry.item))
    }

    /// Retruns the item if it's valid
    pub fn get_mut_by_slot(&mut self, slot: Slot) -> Option<&mut T> {
        let entry = self.entries.get_mut(slot.to_usize())?;
        entry.gen.and(Some(&mut entry.item))
    }

    // TODO: use specific iterator?
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| entry.gen.and(Some(Slot(i as u32))))
    }

    /// Iterator of `(Slot, &T)`
    pub fn enumerate_items(&self) -> impl Iterator<Item = (Slot, &T)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| entry.gen.and(Some((Slot(i as u32), &entry.item))))
    }

    /// Iterator of `(Slot, &mut T)`
    pub fn enumerate_items_mut(&mut self) -> impl Iterator<Item = (Slot, &mut T)> {
        self.entries
            .iter_mut()
            .enumerate()
            .filter_map(|(i, entry)| entry.gen.and(Some((Slot(i as u32), &mut entry.item))))
    }
}
