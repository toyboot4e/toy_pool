/*!
Single-threaded `mpsc` backed by `Rc<RefCell<VecDeque>>`, just for the API
*/

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

type Queue<T> = Rc<RefCell<VecDeque<T>>>;

/// Sender. Often referred to as `tx` (transmission)
#[derive(Debug)]
pub struct Sender<T>(Queue<T>);

impl<T> Sender<T> {
    pub fn send(&self, item: T) {
        self.0.borrow_mut().push_back(item);
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

/// Receiver. Often referred to as `rx` (receiver)
#[derive(Debug)]
pub struct Receiver<T>(Queue<T>);

impl<T> Receiver<T> {
    /// FIFO event listening
    pub fn recv(&self) -> Option<T> {
        self.0.borrow_mut().pop_front()
    }
}

pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let queue = Rc::new(RefCell::new(VecDeque::new()));
    (Sender(queue.clone()), Receiver(queue))
}
