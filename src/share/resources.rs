//! A bounded, in-memory content store. The hub protects resources held by
//! waiting and in-flight comments under the same lock as the queue.

use std::collections::{BTreeMap, HashMap, HashSet};

use bytes::Bytes;

use crate::protocol::resource::{MAX_FILE_BYTES, ResourceId};

pub(crate) const MAX_STORED_BYTES: usize = 128 * 1024 * 1024;
// Allow for map and tree entries, allocation slack and byte handles.
const RESOURCE_OVERHEAD: usize = 1024;

#[derive(Default)]
pub(crate) struct Resources {
    entries: HashMap<ResourceId, Bytes>,
    oldest_first: BTreeMap<u64, ResourceId>,
    order: u64,
    used: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoreError {
    FileTooLarge,
    Full,
}

impl Resources {
    pub(crate) fn contains(&self, id: ResourceId) -> bool {
        self.entries.contains_key(&id)
    }

    pub(crate) fn get(&self, id: ResourceId) -> Option<Bytes> {
        self.entries.get(&id).cloned()
    }

    pub(crate) fn add(
        &mut self,
        id: ResourceId,
        bytes: Bytes,
        protected: impl FnOnce() -> HashSet<ResourceId>,
    ) -> Result<(), StoreError> {
        self.add_with_limit(id, bytes, protected, MAX_STORED_BYTES)
    }

    fn add_with_limit(
        &mut self,
        id: ResourceId,
        bytes: Bytes,
        protected: impl FnOnce() -> HashSet<ResourceId>,
        limit: usize,
    ) -> Result<(), StoreError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(StoreError::FileTooLarge);
        }
        if self.contains(id) {
            return Ok(());
        }
        let charge = cost(&bytes);
        let needed = (self.used + charge).saturating_sub(limit);
        if needed > 0 {
            let protected = protected();
            let mut freed = 0;
            let mut evicted = Vec::new();
            for (order, oldest) in &self.oldest_first {
                if !protected.contains(oldest)
                    && let Some(bytes) = self.entries.get(oldest)
                {
                    freed += cost(bytes);
                    evicted.push((*order, *oldest));
                    if freed >= needed {
                        break;
                    }
                }
            }
            if freed < needed {
                return Err(StoreError::Full);
            }
            for (order, id) in evicted {
                self.oldest_first.remove(&order);
                self.entries.remove(&id);
            }
            self.used -= freed;
            // A wave of small files can leave a large mostly empty table
            // when replaced by a few large files.
            if self.entries.capacity() > 4 * self.entries.len() + 16 {
                self.entries.shrink_to_fit();
            }
        }
        // A body frame may share a larger HTTP read buffer. Own only the
        // file's actual bytes in the store.
        self.entries.insert(id, Bytes::copy_from_slice(&bytes));
        self.oldest_first.insert(self.order, id);
        self.order += 1;
        self.used += charge;
        Ok(())
    }
}

fn cost(bytes: &Bytes) -> usize {
    bytes.len() + RESOURCE_OVERHEAD
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{
        protocol::resource::{MAX_FILE_BYTES, ResourceId},
        share::resources::{RESOURCE_OVERHEAD, Resources, StoreError},
    };

    #[test]
    fn evicts_oldest_unprotected_and_deduplicates() {
        let mut resources = Resources::default();
        let add = |resources: &mut Resources, data: &'static [u8], protected| {
            let id = ResourceId::digest(data);
            resources
                .add_with_limit(
                    id,
                    Bytes::from_static(data),
                    || protected,
                    3 * RESOURCE_OVERHEAD + 32,
                )
                .map(|()| id)
        };
        let first = add(&mut resources, b"oldest", Default::default()).unwrap();
        let second = add(&mut resources, b"second", Default::default()).unwrap();
        let third = add(&mut resources, b"third", Default::default()).unwrap();
        assert_eq!(
            add(&mut resources, b"second", Default::default()),
            Ok(second)
        );
        assert_eq!(
            resources.used,
            3 * RESOURCE_OVERHEAD + b"oldestsecondthird".len()
        );
        assert_eq!(
            add(&mut resources, b"cannot fit", [first, second, third].into()),
            Err(StoreError::Full)
        );
        let fourth = add(&mut resources, b"fourth", [first].into()).unwrap();
        assert!(!resources.contains(second));
        assert!(resources.contains(first));
        assert!(resources.contains(third));
        assert!(resources.contains(fourth));
    }

    #[test]
    fn eviction_releases_table_capacity_left_by_small_files() {
        let mut resources = Resources::default();
        let budget = 128 * (RESOURCE_OVERHEAD + 1);
        for byte in 0..128 {
            let bytes = Bytes::copy_from_slice(&[byte]);
            resources
                .add_with_limit(ResourceId::digest(&bytes), bytes, Default::default, budget)
                .unwrap();
        }
        let large = Bytes::from(vec![0; budget - RESOURCE_OVERHEAD]);
        resources
            .add_with_limit(ResourceId::digest(&large), large, Default::default, budget)
            .unwrap();
        assert_eq!(resources.entries.len(), 1);
        assert!(resources.entries.capacity() <= 16);
        assert_eq!(resources.used, budget);
    }

    #[test]
    fn file_cap() {
        let mut resources = Resources::default();
        let exact = Bytes::from(vec![0; MAX_FILE_BYTES]);
        let id = ResourceId::digest(&exact);
        assert_eq!(
            resources.add_with_limit(id, exact.clone(), Default::default, MAX_FILE_BYTES),
            Err(StoreError::Full),
            "entry overhead counts even when all payload bytes would fit"
        );
        assert_eq!(resources.add(id, exact, Default::default), Ok(()));
        let larger = Bytes::from(vec![0; MAX_FILE_BYTES + 1]);
        assert_eq!(
            resources.add(id, larger, Default::default),
            Err(StoreError::FileTooLarge)
        );
        assert_eq!(resources.used, MAX_FILE_BYTES + RESOURCE_OVERHEAD);
    }
}
