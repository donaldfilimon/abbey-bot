//! Experience replay (`docs/spec/brain.md`, "ReplayBuffer.swift").
//!
//! A fixed-capacity circular buffer: once full, the oldest entry is
//! overwritten, so a long-running gateway session never grows it without
//! bound.

use serde::{Deserialize, Serialize};

use super::nn::Rng;

/// One transition observed by the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Experience {
    pub state: Vec<f32>,
    pub action: usize,
    pub reward: f32,
    pub next_state: Vec<f32>,
    pub done: bool,
}

/// Fixed-capacity circular store of [`Experience`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayBuffer {
    storage: Vec<Experience>,
    write_index: usize,
    capacity: usize,
}

impl ReplayBuffer {
    /// Creates an empty buffer that will hold at most `capacity` entries.
    ///
    /// # Panics
    /// Panics if `capacity == 0` — a buffer that can hold nothing cannot
    /// honour `push`.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "replay buffer capacity must be positive");
        Self {
            storage: Vec::with_capacity(capacity),
            write_index: 0,
            capacity,
        }
    }

    /// Maximum number of entries retained.
    #[must_use]
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of entries currently held (never exceeds `capacity`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Appends while under capacity, otherwise overwrites the oldest entry.
    /// The most recent `n` experiences, oldest first — what a snapshot keeps
    /// so learning continues across a restart instead of starting from an
    /// empty buffer every time.
    pub fn recent(&self, n: usize) -> Vec<Experience> {
        let len = self.storage.len();
        if len == 0 || n == 0 {
            return Vec::new();
        }
        let n = n.min(len);
        // When full, the oldest element sits at `write_index`.
        let start = if len < self.capacity {
            0
        } else {
            self.write_index
        };
        (0..n)
            .map(|i| self.storage[(start + (len - n) + i) % len].clone())
            .collect()
    }

    pub fn push(&mut self, experience: Experience) {
        if self.storage.len() < self.capacity {
            self.storage.push(experience);
        } else {
            self.storage[self.write_index] = experience;
            self.write_index = (self.write_index + 1) % self.capacity;
        }
    }

    /// Uniform sample of `size` entries, with replacement. Returns an empty
    /// vector if the buffer is empty — callers gate on `len` before calling,
    /// but this stays total rather than panicking.
    #[must_use]
    pub fn sample(&self, size: usize, rng: &mut Rng) -> Vec<Experience> {
        if self.is_empty() {
            return Vec::new();
        }
        (0..size)
            .map(|_| self.storage[rng.next_usize_below(self.storage.len())].clone())
            .collect()
    }

    #[cfg(test)]
    /// Drops every entry and resets the write cursor; capacity is kept.
    pub fn clear(&mut self) {
        self.storage.clear();
        self.write_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp(tag: f32) -> Experience {
        Experience {
            state: vec![tag],
            action: 0,
            reward: tag,
            next_state: vec![tag + 1.0],
            done: false,
        }
    }

    #[test]
    fn fills_then_wraps_circularly_without_exceeding_capacity() {
        let mut buf = ReplayBuffer::new(3);
        assert!(buf.is_empty());
        for i in 0..3 {
            buf.push(exp(i as f32));
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(
            buf.storage.iter().map(|e| e.reward).collect::<Vec<_>>(),
            [0.0, 1.0, 2.0]
        );

        buf.push(exp(3.0));
        assert_eq!(buf.len(), 3, "capacity must not be exceeded");
        assert_eq!(
            buf.storage.iter().map(|e| e.reward).collect::<Vec<_>>(),
            [3.0, 1.0, 2.0]
        );

        buf.push(exp(4.0));
        buf.push(exp(5.0));
        buf.push(exp(6.0));
        assert_eq!(buf.len(), 3);
        assert_eq!(
            buf.storage.iter().map(|e| e.reward).collect::<Vec<_>>(),
            [6.0, 4.0, 5.0]
        );
        assert_eq!(buf.capacity(), 3);
    }

    #[test]
    fn recent_returns_oldest_first_even_after_wrap() {
        let mut buf = ReplayBuffer::new(3);
        for i in 0..5u8 {
            buf.push(Experience {
                state: vec![f32::from(i)],
                action: 0,
                reward: 0.0,
                next_state: vec![],
                done: true,
            });
        }
        let recent: Vec<f32> = buf.recent(2).into_iter().map(|e| e.state[0]).collect();
        assert_eq!(recent, [3.0, 4.0], "last two, oldest first");
        let all: Vec<f32> = buf.recent(10).into_iter().map(|e| e.state[0]).collect();
        assert_eq!(all, [2.0, 3.0, 4.0]);
        assert!(ReplayBuffer::new(3).recent(2).is_empty());
    }

    #[test]
    fn sample_on_empty_returns_empty() {
        let buf = ReplayBuffer::new(5);
        let mut rng = Rng::new(1);
        assert!(buf.sample(10, &mut rng).is_empty());
    }

    #[test]
    fn sample_returns_requested_size_from_held_entries() {
        let mut buf = ReplayBuffer::new(4);
        buf.push(exp(1.0));
        buf.push(exp(2.0));
        let mut rng = Rng::new(99);
        let s = buf.sample(50, &mut rng);
        assert_eq!(s.len(), 50, "sampling is with replacement");
        assert!(s.iter().all(|e| e.reward == 1.0 || e.reward == 2.0));
        assert!(s.iter().any(|e| e.reward == 1.0));
        assert!(s.iter().any(|e| e.reward == 2.0));
    }

    #[test]
    fn clear_resets_length_and_cursor() {
        let mut buf = ReplayBuffer::new(2);
        buf.push(exp(1.0));
        buf.push(exp(2.0));
        buf.push(exp(3.0));
        assert_eq!(buf.write_index, 1);
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.write_index, 0);
        buf.push(exp(9.0));
        assert_eq!(buf.storage[0].reward, 9.0);
    }

    #[test]
    fn experience_round_trips_through_serde() {
        let e = exp(2.5);
        let json = serde_json::to_string(&e).expect("serialize");
        let back: Experience = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, e);
    }

    #[test]
    #[should_panic(expected = "capacity must be positive")]
    fn zero_capacity_is_rejected() {
        let _ = ReplayBuffer::new(0);
    }
}
