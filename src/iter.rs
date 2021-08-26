//! Iterator types for the [`Pool`]

use super::*;

pub struct Iter<'a, T: 'static> {
    // TODO: len: u32,
    pub(crate) entries: slice::Iter<'a, PoolEntry<T>>,
}

impl<'a, T: 'static> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.entries.next()?;
            if entry.data.is_none() {
                continue;
            }
            return entry.data.as_ref();
        }
    }
}

pub struct IterMut<'a, T: 'static> {
    // TODO: len: u32,
    pub(crate) entries: slice::IterMut<'a, PoolEntry<T>>,
}

impl<'a, T: 'static> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.entries.next()?;
            if entry.data.is_none() {
                continue;
            }
            return entry.data.as_mut();
        }
    }
}

impl<'a, T: 'static> IntoIterator for &'a Pool<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        Iter {
            entries: self.entries.iter(),
        }
    }
}

impl<'a, T: 'static> IntoIterator for &'a mut Pool<T> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        IterMut {
            entries: self.entries.iter_mut(),
        }
    }
}
