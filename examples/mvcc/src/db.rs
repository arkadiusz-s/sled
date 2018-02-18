use std::sync::{Arc, RwLock};

use sled::{Config, DbResult, Tree};

use super::*;

// persist the TS every TS_SAFETY_BUFFER / 2 txns.
// every time we start, bump TS by this much.
// 64 bits = 6 billion per second for 100 years,
// so, no big deal, unless...
const TS_SAFETY_BUFFER: u64 = 4294967296;

// where to store the TS every once in a while
const TS_PERSIST_KEY: &'static [u8] = b"tx_persist";

pub struct Db {
    tree: sled::Tree,
    ts: AtomicUsize,
    mvcc: Mvcc,
}

impl Db {
    pub fn start(config: Config) -> DbResult<Db, ()> {
        let tree = Tree::start(config)?;
        let last_ts_v = tree.get(TS_PERSIST_KEY)?;
        let last_ts = if let Some(last_ts_bytes) = last_ts_v {
            assert_eq!(
                last_ts_bytes.len(),
                8,
                "last known transaction bytes are corrupted"
            );

            bytes_to_ts(&*last_ts_bytes)
        } else {
            0
        };

        let bumped_ts: u64 = last_ts + TS_SAFETY_BUFFER;

        tree.set(TS_PERSIST_KEY.to_vec(), ts_to_bytes(bumped_ts))?;

        let db = Db {
            tree: tree,
            ts: AtomicUsize::new(bumped_ts as usize),
            mvcc: Mvcc::default(),
        };

        db.recover()?;

        Ok(db)
    }

    // bump stored ts by TS_SAFETY_BUFFER
    // for (ts, writeset) in ts range:
    //   for Wts, Version in writeset:
    //     filter @k, remove (Wts, Version)
    // delete ts from sled
    fn recover(&self) -> DbResult<(), ()> {
        for res in self.tree.scan(b"!") {
            let (writeset_key, writeset_bytes) = res?;
            if writeset_key[0] != b"!"[0] {
                return Ok(());
            }

            assert_eq!(
                writeset_key.len(),
                9,
                "transaction key must be 9 bytes long"
            );

            let wts = bytes_to_ts(&writeset_key[1..9]);

            let writeset: WriteSet = deserialize(&*writeset_bytes).expect(
                "corrupt transaction data found",
            );

            for key in writeset {
                let padded_key = key_safety_pad(&key);
                let value_opt = self.tree.get(&padded_key)?;

                if let Some(value) = value_opt {
                    let mut versions: Versions =
                        deserialize(&*value).expect("corrupt Data found");

                    let mut pruned = false;
                    for &(ts, version) in &versions {
                        if ts == wts {
                            self.tree.del(&*ts_to_bytes(version))?;
                            pruned = true;
                        }
                    }

                    if pruned {
                        versions.retain(|&(ts, version)| ts != wts);
                        let new_value = if versions.is_empty() {
                            None
                        } else {
                            Some(serialize(&versions, Infinite).unwrap())
                        };
                        self.tree.cas(padded_key, Some(value), new_value);
                    }
                }
            }

            self.tree.del(&writeset_key)?;
        }

        Ok(())
    }

    // bump timestamp and possibly persist a boosted version
    pub(super) fn ts(&self, n: usize) -> Ts {
        let ret = self.ts.fetch_add(n, Relaxed) as Ts;

        // if we need to boost the persisted TS, do it
        if ret % TS_SAFETY_BUFFER > (TS_SAFETY_BUFFER * 3 / 4) {
            let last = (ret / TS_SAFETY_BUFFER) * TS_SAFETY_BUFFER;
            let next = ((ret / TS_SAFETY_BUFFER) + 1) * TS_SAFETY_BUFFER;
            if self.ts.compare_and_swap(
                ret as usize + n,
                next as usize,
                Relaxed,
            ) == ret as usize + n
            {
                self.tree
                    .cas(
                        TS_PERSIST_KEY.to_vec(),
                        Some(ts_to_bytes(last)),
                        Some(ts_to_bytes(next)),
                    )
                    .unwrap();
            }
        }
        ret
    }

    /// create a new transaction
    pub fn tx<'a>(&'a self) -> Tx<'a> {
        Tx {
            predicates: vec![],
            gets: vec![],
            sets: vec![],
            deletes: vec![],
            db: &self,
        }
    }

    pub(super) fn read(&self, k: &Key) -> Arc<RwLock<Chain>> {
        if let Some(chain) = self.mvcc.get(k) {
            chain
        } else {
            // pull a key out of the tree, or represent its absence
            let wrapped_key = key_safety_pad(k);

            if let Some(found) = self.tree.get(&wrapped_key).unwrap() {
                let versions: Versions =
                    deserialize(&*found).expect("corrupt Data found");

                let chain: Vec<_> = versions
                    .into_iter()
                    .map(|(wts, version)| {
                        MemRecord {
                            rts: AtomicUsize::new(0),
                            wts: wts,
                            data: Some(version),
                            // we know this is committed because
                            // during recovery we deleted all pending
                            // versions.
                            status: Mutex::new(Status::Committed),
                            status_cond: Condvar::new(),
                        }
                    })
                    .collect();

                self.mvcc.insert(k.clone(), chain);
                self.mvcc.get(k).unwrap()
            } else {
                self.mvcc.insert(k.clone(), vec![MemRecord::default()]);
                self.mvcc.get(k).unwrap()
            }
        }
    }
}