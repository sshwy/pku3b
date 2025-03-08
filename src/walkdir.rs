use std::collections::LinkedList;
use std::fs::{DirEntry, ReadDir};

type Item = Result<DirEntry, std::io::Error>;
pub struct Iter {
    /// in post-order traversal, we also store the dir entry in the stack.
    stack: LinkedList<(ReadDir, Option<Item>)>,
    /// preorder or postorder
    preorder: bool,
}

impl Iterator for Iter {
    type Item = Item;

    fn next(&mut self) -> Option<Self::Item> {
        fn try_read_dir(e: &Item) -> Option<ReadDir> {
            if let Ok(e) = &e {
                let path = e.path();
                if path.is_dir() {
                    // if the directory is readable, push it to the stack.
                    // otherwise, ignore it
                    if let Ok(dir) = path.read_dir() {
                        return Some(dir);
                    }
                }
            }
            None
        }
        while let Some((d, _)) = self.stack.back_mut() {
            if let Some(e) = d.next() {
                // if the entry is a readable directory, push it to the stack.
                // otherwise it is considered as a leafy entry.
                if let Some(dir) = try_read_dir(&e) {
                    if self.preorder {
                        self.stack.push_back((dir, None));
                        return Some(e);
                    } else {
                        // e is a directory, which is not the item we want to return.
                        // so we store it and get the next item recursively.
                        self.stack.push_back((dir, Some(e)));
                        return self.next();
                    }
                }

                return Some(e);
            }

            // no more entries in this directory, so we drop d
            if let Some((_, Some(e))) = self.stack.pop_back() {
                // we only store the directory entry in post-order traversal
                assert!(!self.preorder);
                return Some(e);
            }
        }
        // no more directories in the stack
        None
    }
}

impl futures_util::Stream for Iter {
    type Item = Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.next())
    }
}

pub fn walkdir(dir: ReadDir, preorder: bool) -> futures_util::stream::Iter<Iter> {
    futures_util::stream::iter(Iter {
        stack: LinkedList::from([(dir, None)]),
        preorder,
    })
}
