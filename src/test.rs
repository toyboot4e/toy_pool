use std::mem;

use super::*;

#[test]
fn size() {
    assert_eq!(
        mem::size_of::<Handle<()>>(),
        mem::size_of::<u64>() + mem::size_of::<smpsc::Sender<Message>>(),
    );

    // TODO: test pool entry size
}
