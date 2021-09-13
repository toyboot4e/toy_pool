/*!
Tree support
*/

mod link;

// TODO use a nonmax type for slots

use crate::{tree::link::Link, *};

pub type NodeHandle<T> = Handle<Node<T>>;

pub struct Tree<T> {
    nodes: Pool<Node<T>>,
    root: Link<Slot>,
}

impl<T> link::Tree for Tree<T> {
    type Slot = Slot;
    type Id = NodeHandle<T>;

    fn root_mut(&mut self) -> &mut Link<Self::Slot> {
        &mut self.root
    }

    fn link_mut_by_slot(&mut self, slot: Self::Slot) -> Option<&mut Link<Self::Slot>> {
        self.nodes.get_mut_by_slot(slot).map(|n| &mut n.link)
    }

    fn link2_mut_by_slot(
        &mut self,
        s0: Self::Slot,
        s1: Self::Slot,
    ) -> Option<(&mut Link<Self::Slot>, &mut Link<Self::Slot>)> {
        self.nodes
            .get2_mut_by_slot(s0, s1)
            .map(|(n0, n1)| (&mut n0.link, &mut n1.link))
    }

    // TODO: consider cheaper API
    fn link_mut_by_id(&mut self, id: Self::Id) -> Option<&mut Link<Self::Slot>> {
        // we know it's alive since we're using a strong handle
        Some(&mut self.nodes[&id].link)
    }
}

impl<T> link::Id<Slot> for NodeHandle<T> {
    fn slot(&self) -> Slot {
        self.slot
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Node<T> {
    data: T,
    link: Link<Slot>,
}

impl<T> Tree<T> {
    pub fn node_by_slot(&self, slot: Slot) -> Option<&Node<T>> {
        self.nodes
            .entries
            .get(slot.to_usize())
            .and_then(|entry| entry.data.as_ref())
    }

    pub fn node_mut_by_slot(&mut self, slot: Slot) -> Option<&mut Node<T>> {
        self.nodes
            .entries
            .get_mut(slot.to_usize())
            .and_then(|entry| entry.data.as_mut())
    }

    pub fn data_by_slot(&self, slot: Slot) -> Option<&T> {
        self.node_by_slot(slot).map(|n| &n.data)
    }

    pub fn data_mut_by_slot(&mut self, slot: Slot) -> Option<&mut T> {
        self.node_mut_by_slot(slot).map(|n| &mut n.data)
    }

    pub fn insert(&mut self, item: impl Into<T>) -> Handle<Node<T>> {
        let node = Node::root(item.into());

        todo!()
    }
}

impl<T> Node<T> {
    pub fn root(data: T) -> Self {
        Self {
            data,
            link: Default::default(),
        }
    }
}

impl<T> ops::Index<&Handle<Node<T>>> for Tree<T> {
    type Output = Node<T>;
    fn index(&self, handle: &Handle<Node<T>>) -> &Self::Output {
        &self.nodes[handle]
    }
}

impl<T> ops::IndexMut<&Handle<Node<T>>> for Tree<T> {
    fn index_mut(&mut self, handle: &Handle<Node<T>>) -> &mut Self::Output {
        &mut self.nodes[handle]
    }
}
