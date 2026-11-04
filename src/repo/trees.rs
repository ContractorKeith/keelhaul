//! Bounded parsed-tree cache for readers that must enumerate every path.
use super::{Error, blobs::BlobStore};
use crate::{
    backend::Backend,
    format::{pack::BlobKind, tree::Tree},
};
use std::{
    collections::{HashMap, VecDeque},
    rc::Rc,
};
const LIMIT: usize = 64 * 1024 * 1024;
const ENTRIES: usize = 4096;
#[derive(Clone)]
struct Cached {
    tree: Rc<Tree>,
    problems: Vec<String>,
}
#[derive(Clone)]
struct Broken {
    object: String,
    problem: &'static str,
    detail: String,
}
type Saved = Result<Cached, Broken>;
#[derive(Default)]
pub struct TreeCache {
    entries: HashMap<(String, bool), (Saved, usize)>,
    order: VecDeque<(String, bool)>,
    bytes: usize,
}
impl TreeCache {
    pub fn read<B: Backend>(
        &mut self,
        store: &mut BlobStore<'_, B>,
        id: &str,
    ) -> Result<Rc<Tree>, Error> {
        self.get(store, id, false).map(|c| c.tree)
    }
    pub fn salvage<B: Backend>(
        &mut self,
        store: &mut BlobStore<'_, B>,
        id: &str,
    ) -> Result<(Rc<Tree>, Vec<String>), Error> {
        self.get(store, id, true).map(|c| (c.tree, c.problems))
    }
    fn get<B: Backend>(
        &mut self,
        store: &mut BlobStore<'_, B>,
        id: &str,
        salvage: bool,
    ) -> Result<Cached, Error> {
        let key = (id.to_owned(), salvage);
        if let Some((saved, _)) = self.entries.get(&key) {
            return restore(saved.clone());
        }
        let mut cost = 0;
        let result = store.read(BlobKind::Tree, id).and_then(|bytes| {
            // Bound cached allocations conservatively, as well as entry count.
            cost = bytes.len().saturating_mul(8);
            let parsed = if salvage {
                Tree::salvage(&bytes)
            } else {
                Tree::parse(&bytes).map(|t| (t, vec![]))
            };
            parsed
                .map(|(tree, problems)| Cached {
                    tree: Rc::new(tree),
                    problems,
                })
                .map_err(|e| Error::Damage {
                    object: id.into(),
                    problem: "bad-format",
                    detail: e.to_string(),
                })
        });
        let saved = match result {
            Ok(c) => Ok(c),
            Err(Error::Damage {
                object,
                problem,
                detail,
            }) => {
                cost = object.len() + detail.len();
                Err(Broken {
                    object,
                    problem,
                    detail,
                })
            }
            Err(e) => return Err(e),
        };
        if cost <= LIMIT {
            while self.bytes.saturating_add(cost) > LIMIT || self.entries.len() >= ENTRIES {
                let Some(old) = self.order.pop_front() else {
                    break;
                };
                if let Some((_, cost)) = self.entries.remove(&old) {
                    self.bytes -= cost;
                }
            }
            self.bytes += cost;
            self.order.push_back(key.clone());
            self.entries.insert(key, (saved.clone(), cost));
        }
        restore(saved)
    }
}
fn restore(saved: Saved) -> Result<Cached, Error> {
    saved.map_err(|e| Error::Damage {
        object: e.object,
        problem: e.problem,
        detail: e.detail,
    })
}
