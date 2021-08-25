/*!
Single-threaded `mpsc` backed by `Rc<RefCell<Vec>>`, just for the API

It's using `Vec` so events are received in reverse order.
*/

use std::{cell::RefCell, rc::Rc};

type Queue<T> = Rc<RefCell<Vec<T>>>;

/// Sender. Often referred to as `tx` (transmission)
#[derive(Debug)]
pub struct Sender<T>(Queue<T>);

impl<T> Sender<T> {
    pub fn send(&self, item: T) {
        self.0.borrow_mut().push(item);
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
    pub fn recv(&self) -> Option<T> {
        self.0.borrow_mut().pop()
    }
}

pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    let queue = Rc::new(RefCell::new(Vec::new()));
    (Sender(queue.clone()), Receiver(queue))
}
