//! Play queue: shuffle bag (no repeats until every clip has played), history, bad-clip skipping.
use std::collections::HashSet;

pub struct Queue {
    n: usize,
    pool: Vec<usize>,
    bad: HashSet<usize>,
    shuffle: bool,
    hist: Vec<usize>,
    hpos: usize,
    bag: Vec<usize>,
    pending: Option<usize>,
    rng: u64,
}

const HIST_CAP: usize = 1000;

impl Queue {
    pub fn new(n: usize, shuffle: bool, seed: u64) -> Queue {
        Queue {
            n,
            pool: (0..n).collect(),
            bad: HashSet::new(),
            shuffle,
            hist: vec![],
            hpos: 0,
            bag: vec![],
            pending: None,
            rng: seed | 1,
        }
    }
    fn rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn len(&self) -> usize {
        self.n
    }
    /// Restrict playback to these indices (e.g. only cached clips while offline).
    pub fn set_pool(&mut self, mut pool: Vec<usize>) {
        pool.retain(|&i| i < self.n);
        pool.sort_unstable();
        pool.dedup();
        self.pool = pool;
        self.bag.clear();
        self.pending = None;
    }
    pub fn pool_len(&self) -> usize {
        self.pool.iter().filter(|i| !self.bad.contains(i)).count()
    }
    pub fn current(&self) -> Option<usize> {
        self.hist.get(self.hpos).copied()
    }
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }
    pub fn set_shuffle(&mut self, on: bool) {
        if on != self.shuffle {
            self.shuffle = on;
            self.hist.truncate(self.hpos + 1);
            self.bag.clear();
            self.pending = None;
        }
    }
    pub fn mark_bad(&mut self, idx: usize) {
        self.bad.insert(idx);
        if self.pending == Some(idx) {
            self.pending = None;
        }
        let cur = self.hpos;
        let mut k = 0;
        self.hist.retain(|&h| {
            let keep = k <= cur || h != idx;
            k += 1;
            keep
        });
    }
    pub fn clear_bad(&mut self) {
        self.bad.clear();
    }
    fn ok(&self, i: usize) -> bool {
        !self.bad.contains(&i) && self.pool.binary_search(&i).is_ok()
    }
    fn pick_new(&mut self) -> Option<usize> {
        let cur = self.current();
        let cands: Vec<usize> = self
            .pool
            .iter()
            .copied()
            .filter(|i| !self.bad.contains(i))
            .collect();
        if cands.is_empty() {
            return None;
        }
        if cands.len() == 1 {
            return Some(cands[0]);
        }
        if !self.shuffle {
            let after = cur.and_then(|c| cands.iter().copied().find(|&i| i > c));
            return Some(after.unwrap_or(cands[0]));
        }
        loop {
            while let Some(i) = self.bag.pop() {
                if self.ok(i) && Some(i) != cur {
                    return Some(i);
                }
            }
            let mut b = cands.clone();
            for k in (1..b.len()).rev() {
                let j = (self.rand() % (k as u64 + 1)) as usize;
                b.swap(k, j);
            }
            self.bag = b;
        }
    }
    pub fn peek_next(&mut self) -> Option<usize> {
        while self.hpos + 1 < self.hist.len() {
            let f = self.hist[self.hpos + 1];
            if self.ok(f) {
                return Some(f);
            }
            self.hist.remove(self.hpos + 1);
        }
        if self.pending.is_none() {
            self.pending = self.pick_new();
        }
        self.pending
    }
    pub fn next(&mut self) -> Option<usize> {
        let p = self.peek_next()?;
        if self.hpos + 1 < self.hist.len() {
            self.hpos += 1;
        } else {
            self.push(p);
        }
        self.pending = None;
        Some(p)
    }
    fn push(&mut self, i: usize) {
        if !self.hist.is_empty() {
            self.hist.truncate(self.hpos + 1);
        }
        self.hist.push(i);
        if self.hist.len() > HIST_CAP {
            self.hist.remove(0);
        }
        self.hpos = self.hist.len() - 1;
    }
    pub fn prev(&mut self) -> Option<usize> {
        while self.hpos > 0 {
            self.hpos -= 1;
            let i = self.hist[self.hpos];
            if self.ok(i) {
                return Some(i);
            }
        }
        None
    }
    /// Play a specific clip now (grid selection); drops any forward history.
    pub fn jump(&mut self, idx: usize) {
        if idx >= self.n {
            return;
        }
        self.bad.remove(&idx);
        self.push(idx);
        self.pending = None;
        self.bag.retain(|&b| b != idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shuffle_covers_all_before_repeating() {
        let mut q = Queue::new(20, true, 42);
        let mut seen = HashSet::new();
        let first = q.next().unwrap();
        seen.insert(first);
        for _ in 0..19 {
            assert!(seen.insert(q.next().unwrap()), "repeat inside one bag");
        }
        assert_eq!(seen.len(), 20);
        // second bag starts without immediately repeating the last clip
        let last = q.current().unwrap();
        assert_ne!(q.next().unwrap(), last);
    }
    #[test]
    fn sequential_wraps() {
        let mut q = Queue::new(3, false, 1);
        let v: Vec<_> = (0..7).map(|_| q.next().unwrap()).collect();
        assert_eq!(v, vec![0, 1, 2, 0, 1, 2, 0]);
    }
    #[test]
    fn peek_matches_next_and_history() {
        let mut q = Queue::new(10, true, 7);
        let a = q.next().unwrap();
        let p = q.peek_next().unwrap();
        assert_eq!(q.peek_next(), Some(p), "peek is stable");
        let b = q.next().unwrap();
        assert_eq!(p, b);
        assert_eq!(q.prev(), Some(a));
        assert_eq!(q.current(), Some(a));
        assert_eq!(q.next(), Some(b), "forward history is replayed");
        assert_eq!(q.prev(), Some(a));
        assert_eq!(q.prev(), None, "start of history");
    }
    #[test]
    fn bad_clips_are_skipped() {
        let mut q = Queue::new(5, false, 1);
        q.mark_bad(1);
        q.mark_bad(2);
        let v: Vec<_> = (0..4).map(|_| q.next().unwrap()).collect();
        assert_eq!(v, vec![0, 3, 4, 0]);
        let mut q = Queue::new(5, true, 3);
        for i in 0..4 {
            q.mark_bad(i);
        }
        assert_eq!(q.next(), Some(4));
        assert_eq!(q.next(), Some(4));
        q.mark_bad(4);
        assert_eq!(q.next(), None);
        q.clear_bad();
        assert!(q.next().is_some());
    }
    #[test]
    fn marking_pending_bad_changes_peek() {
        let mut q = Queue::new(6, true, 9);
        q.next();
        let p = q.peek_next().unwrap();
        q.mark_bad(p);
        assert_ne!(q.peek_next(), Some(p));
    }
    #[test]
    fn pool_restriction_and_jump() {
        let mut q = Queue::new(10, true, 5);
        q.set_pool(vec![2, 4, 6]);
        for _ in 0..30 {
            assert!([2, 4, 6].contains(&q.next().unwrap()));
        }
        q.jump(9);
        assert_eq!(q.current(), Some(9));
        assert_eq!(q.prev().map(|i| [2, 4, 6].contains(&i)), Some(true));
        q.set_pool(vec![]);
        assert_eq!(q.next(), None);
    }
    #[test]
    fn toggle_shuffle_drops_forward_history() {
        let mut q = Queue::new(8, true, 11);
        q.next();
        q.next();
        q.prev();
        q.set_shuffle(false);
        assert!(!q.shuffle());
        let c = q.current().unwrap();
        assert_eq!(q.next(), Some((c + 1) % 8));
    }
    #[test]
    fn empty_queue() {
        let mut q = Queue::new(0, true, 1);
        assert_eq!(q.next(), None);
        assert_eq!(q.prev(), None);
        assert_eq!(q.current(), None);
    }
}
