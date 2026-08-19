//! Deep Q-learning agent (`docs/spec/brain.md`, "DQNAgent.swift").
//!
//! A plain struct rather than an actor: the caller owns it behind whatever
//! lock it needs (the Discord layer holds one per guild behind a `Mutex`).
//! Nothing here touches I/O; randomness comes from the agent's own seeded
//! [`Rng`], so every test is reproducible.
//!
//! Open decision, carried over verbatim from the spec: the networks are built
//! with [`OutputActivation::Linear`]. Q-values must keep their magnitude and
//! sign for the Bellman target `reward + γ·max Q(s', a')` to mean anything; a
//! softmaxed output collapses them onto a probability simplex and training
//! silently never converges. If Abbey's DQN was ever tuned against softmaxed
//! Q-values, switching to `Linear` changes behaviour — that is Donald's call,
//! which is why the activation is an explicit choice rather than a silent fix.

use serde::{Deserialize, Serialize};

use super::nn::{NeuralNetwork, OutputActivation, Rng};
use super::replay::{Experience, ReplayBuffer};

/// Discount factor on future reward.
const GAMMA: f32 = 0.99;
/// Initial ε-greedy exploration rate.
const EPSILON_INITIAL: f32 = 0.1;
/// Floor the decay never crosses.
const EPSILON_MIN: f32 = 0.01;
/// Multiplicative decay applied once per `learn()`.
const EPSILON_DECAY: f32 = 0.995;
/// Experiences sampled per `learn()`; below this many held, `learn()` is a no-op.
const BATCH_SIZE: usize = 64;
/// `learn()` calls between hard target-network syncs.
const TARGET_SYNC_INTERVAL: u64 = 100;
/// SGD step size (the Swift `train` default).
const LEARNING_RATE: f32 = 0.001;

/// Serialisable weights + exploration state — without this the agent
/// relearns from scratch on every restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrainSnapshot {
    pub topology: Vec<usize>,
    pub layers: Vec<LayerSnapshot>,
    pub epsilon: f32,
    pub step_count: u64,
    /// The most recent experiences (up to [`SNAPSHOT_EXPERIENCES`]), oldest
    /// first, so a restart resumes learning from a warm buffer instead of an
    /// empty one. Absent in older snapshots.
    #[serde(default)]
    pub experiences: Vec<Experience>,
}

/// How many replay experiences a snapshot keeps.
pub const SNAPSHOT_EXPERIENCES: usize = 1_000;

/// One dense layer's parameters as stored in a [`BrainSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerSnapshot {
    pub weights: Vec<f32>,
    pub biases: Vec<f32>,
}

/// Why [`DqnAgent::import_weights`] refused a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The snapshot was trained on a different topology; loading it would
    /// silently misalign every weight.
    TopologyMismatch {
        expected: Vec<usize>,
        found: Vec<usize>,
    },
    /// A layer's weight or bias vector is the wrong length for its slot.
    LayerShapeMismatch { layer: usize },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopologyMismatch { expected, found } => {
                write!(
                    f,
                    "snapshot topology {found:?} does not match agent topology {expected:?}"
                )
            }
            Self::LayerShapeMismatch { layer } => {
                write!(
                    f,
                    "snapshot layer {layer} has the wrong number of weights or biases"
                )
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// ε-greedy DQN with an online and a hard-synced target network.
#[derive(Debug, Clone)]
pub struct DqnAgent {
    online: NeuralNetwork,
    target: NeuralNetwork,
    buffer: ReplayBuffer,
    epsilon: f32,
    step_count: u64,
    rng: Rng,
}

impl DqnAgent {
    /// Builds an agent whose networks follow `topology` (input width first,
    /// action count last), with a replay buffer of `buffer_capacity`, and a
    /// PRNG seeded from `seed`. The target network starts as an exact copy of
    /// the online one.
    ///
    /// # Panics
    /// Panics if `topology` has fewer than two entries or a zero width, or if
    /// `buffer_capacity == 0`.
    #[must_use]
    pub fn new(topology: &[usize], buffer_capacity: usize, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        // Linear — Q-values must keep magnitude and sign. See the module doc.
        let online = NeuralNetwork::new(topology, OutputActivation::Linear, &mut rng);
        let target = online.clone();
        Self {
            online,
            target,
            buffer: ReplayBuffer::new(buffer_capacity),
            epsilon: EPSILON_INITIAL,
            step_count: 0,
            rng,
        }
    }

    /// Number of actions — the width of the output layer.
    #[must_use]
    pub fn action_count(&self) -> usize {
        self.online.output_count()
    }

    /// Width of the state vector the agent expects.
    #[must_use]
    pub fn state_size(&self) -> usize {
        self.online.input_count()
    }

    /// Current exploration rate.
    #[must_use]
    pub fn epsilon(&self) -> f32 {
        self.epsilon
    }

    /// Overrides the exploration rate (the multi-guild configuration exposes
    /// this). Clamped to `[0, 1]`.
    pub fn set_epsilon(&mut self, epsilon: f32) {
        self.epsilon = epsilon.clamp(0.0, 1.0);
    }

    /// Number of completed `learn()` calls.
    #[must_use]
    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Number of experiences currently held in the replay buffer.
    #[must_use]
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// Q-values for `state` from the online network.
    #[must_use]
    pub fn q_values(&self, state: &[f32]) -> Vec<f32> {
        self.online.forward(state)
    }

    /// ε-greedy: with probability ε a uniformly random action, otherwise the
    /// argmax of the online network's Q-values (ties go to the lowest index).
    pub fn select_action(&mut self, state: &[f32]) -> usize {
        if self.rng.next_f32() < self.epsilon {
            return self.rng.next_usize_below(self.action_count());
        }
        argmax(&self.online.forward(state))
    }

    /// Stores a transition for later replay.
    pub fn remember(&mut self, experience: Experience) {
        self.buffer.push(experience);
    }

    /// One learning step: replays a batch, hard-syncs the target network
    /// every [`TARGET_SYNC_INTERVAL`] steps, and decays ε. A no-op while the
    /// buffer holds fewer than [`BATCH_SIZE`] experiences.
    pub fn learn(&mut self) {
        if self.buffer.len() < BATCH_SIZE {
            return;
        }

        for exp in self.buffer.sample(BATCH_SIZE, &mut self.rng) {
            // Terminal states bootstrap nothing — their future value is 0 by definition.
            let future_q = if exp.done {
                0.0
            } else {
                self.target
                    .forward(&exp.next_state)
                    .into_iter()
                    .reduce(f32::max)
                    .unwrap_or(0.0)
            };
            let target_q = GAMMA.mul_add(future_q, exp.reward);
            let predicted = self.online.forward(&exp.state);
            let target = make_target(predicted, exp.action, target_q);
            self.online.train(&exp.state, &target, LEARNING_RATE);
        }

        self.step_count += 1;
        if self.step_count.is_multiple_of(TARGET_SYNC_INTERVAL) {
            self.target = self.online.clone();
        }
        self.epsilon = EPSILON_MIN.max(self.epsilon * EPSILON_DECAY);
    }

    /// Copies the online network's parameters plus exploration state.
    #[must_use]
    pub fn export_weights(&self) -> BrainSnapshot {
        BrainSnapshot {
            topology: self.online.topology.clone(),
            layers: self
                .online
                .layers
                .iter()
                .map(|l| LayerSnapshot {
                    weights: l.weights.clone(),
                    biases: l.biases.clone(),
                })
                .collect(),
            epsilon: self.epsilon,
            step_count: self.step_count,
            experiences: self.buffer.recent(SNAPSHOT_EXPERIENCES),
        }
    }

    /// Loads a snapshot into both networks and restores ε / step count, and
    /// refills the replay buffer with the snapshot's recent experiences
    /// (state width must match; others are skipped).
    ///
    /// # Errors
    /// Returns [`ImportError`] — and leaves the agent untouched — if the
    /// snapshot's topology or any layer's shape differs from this agent's.
    pub fn import_weights(&mut self, snapshot: &BrainSnapshot) -> Result<(), ImportError> {
        if snapshot.topology != self.online.topology {
            return Err(ImportError::TopologyMismatch {
                expected: self.online.topology.clone(),
                found: snapshot.topology.clone(),
            });
        }
        if snapshot.layers.len() != self.online.layers.len() {
            return Err(ImportError::LayerShapeMismatch {
                layer: snapshot.layers.len().min(self.online.layers.len()),
            });
        }
        for (idx, (saved, live)) in snapshot.layers.iter().zip(&self.online.layers).enumerate() {
            if saved.weights.len() != live.weights.len() || saved.biases.len() != live.biases.len()
            {
                return Err(ImportError::LayerShapeMismatch { layer: idx });
            }
        }

        for (saved, live) in snapshot.layers.iter().zip(&mut self.online.layers) {
            live.weights.clone_from(&saved.weights);
            live.biases.clone_from(&saved.biases);
        }
        self.target = self.online.clone();
        self.epsilon = snapshot.epsilon;
        self.step_count = snapshot.step_count;
        let width = self.state_size();
        for exp in &snapshot.experiences {
            if exp.state.len() == width && exp.next_state.len() == width {
                self.buffer.push(exp.clone());
            }
        }
        Ok(())
    }
}

/// Index of the largest value; ties resolve to the lowest index. Returns 0
/// for an empty slice, which cannot occur for a well-formed network.
fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .fold(None, |best: Option<(usize, f32)>, (i, &v)| match best {
            Some((_, bv)) if bv >= v => best,
            _ => Some((i, v)),
        })
        .map_or(0, |(i, _)| i)
}

/// Copy the prediction, then overwrite only the taken action's slot.
/// Untouched slots yield zero error, so no gradient flows for actions that
/// were not taken.
fn make_target(mut predicted: Vec<f32>, action: usize, value: f32) -> Vec<f32> {
    predicted[action] = value;
    predicted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bandit_exp(action: usize) -> Experience {
        Experience {
            state: vec![1.0],
            action,
            reward: if action == 1 { 1.0 } else { 0.0 },
            next_state: vec![1.0],
            done: true,
        }
    }

    #[test]
    fn new_agent_reports_shape_and_defaults() {
        let agent = DqnAgent::new(&[128, 64, 32, 3], 10_000, 1);
        assert_eq!(agent.action_count(), 3);
        assert_eq!(agent.state_size(), 128);
        assert_eq!(agent.epsilon(), EPSILON_INITIAL);
        assert_eq!(agent.step_count(), 0);
        assert_eq!(agent.buffer_len(), 0);
        assert_eq!(agent.online, agent.target, "target starts as an exact copy");
        assert_eq!(agent.online.output_activation, OutputActivation::Linear);
    }

    #[test]
    fn select_action_is_within_range_and_greedy_at_zero_epsilon() {
        let mut agent = DqnAgent::new(&[2, 4, 3], 10, 5);
        for _ in 0..200 {
            assert!(agent.select_action(&[0.5, -0.5]) < 3);
        }
        agent.set_epsilon(0.0);
        let q = agent.q_values(&[0.5, -0.5]);
        let expected = argmax(&q);
        for _ in 0..20 {
            assert_eq!(agent.select_action(&[0.5, -0.5]), expected);
        }
    }

    #[test]
    fn set_epsilon_clamps_to_unit_interval() {
        let mut agent = DqnAgent::new(&[1, 2], 10, 0);
        agent.set_epsilon(7.0);
        assert_eq!(agent.epsilon(), 1.0);
        agent.set_epsilon(-3.0);
        assert_eq!(agent.epsilon(), 0.0);
        agent.set_epsilon(0.3);
        assert_eq!(agent.epsilon(), 0.3);
    }

    #[test]
    fn argmax_prefers_lowest_index_on_ties() {
        assert_eq!(argmax(&[1.0, 3.0, 3.0]), 1);
        assert_eq!(argmax(&[-1.0, -2.0]), 0);
        assert_eq!(argmax(&[]), 0);
    }

    #[test]
    fn make_target_overwrites_only_the_taken_slot() {
        let t = make_target(vec![0.1, 0.2, 0.3], 1, 9.0);
        assert_eq!(t, vec![0.1, 9.0, 0.3]);
    }

    #[test]
    fn learn_is_a_noop_under_batch_size() {
        let mut agent = DqnAgent::new(&[1, 4, 3], 100, 3);
        let before = agent.export_weights();
        for i in 0..(BATCH_SIZE - 1) {
            agent.remember(bandit_exp(i % 3));
        }
        agent.learn();
        assert_eq!(agent.step_count(), 0);
        assert_eq!(agent.epsilon(), EPSILON_INITIAL);
        // Weights untouched (the snapshot now also carries the buffer, so
        // compare the learned parts only).
        assert_eq!(agent.export_weights().layers, before.layers);

        agent.remember(bandit_exp(0));
        agent.learn();
        assert_eq!(agent.step_count(), 1);
        assert!(agent.epsilon() < EPSILON_INITIAL);
        assert_ne!(agent.export_weights(), before);
    }

    #[test]
    fn bandit_converges_to_the_rewarded_action() {
        let mut agent = DqnAgent::new(&[1, 8, 3], 1_000, 2024);
        for i in 0..300 {
            agent.remember(bandit_exp(i % 3));
        }
        for _ in 0..300 {
            agent.learn();
        }
        agent.set_epsilon(0.0);
        let q = agent.q_values(&[1.0]);
        assert_eq!(agent.select_action(&[1.0]), 1, "q-values were {q:?}");
        assert!(q[1] > 0.5, "q-values were {q:?}");
        assert!(q[0] < q[1] && q[2] < q[1], "q-values were {q:?}");
    }

    #[test]
    fn epsilon_decays_to_the_floor() {
        let mut agent = DqnAgent::new(&[1, 2, 2], 100, 8);
        for i in 0..BATCH_SIZE {
            agent.remember(bandit_exp(i % 2));
        }
        let mut last = agent.epsilon();
        for _ in 0..1_000 {
            agent.learn();
            assert!(agent.epsilon() <= last);
            assert!(agent.epsilon() >= EPSILON_MIN);
            last = agent.epsilon();
        }
        assert_eq!(agent.epsilon(), EPSILON_MIN);
        assert_eq!(agent.step_count(), 1_000);
    }

    #[test]
    fn target_syncs_hard_every_interval() {
        let mut agent = DqnAgent::new(&[1, 3, 2], 100, 4);
        for i in 0..BATCH_SIZE {
            agent.remember(bandit_exp(i % 2));
        }
        for _ in 0..(TARGET_SYNC_INTERVAL - 1) {
            agent.learn();
        }
        assert_ne!(
            agent.online, agent.target,
            "target lags until the sync step"
        );
        agent.learn();
        assert_eq!(agent.step_count(), TARGET_SYNC_INTERVAL);
        assert_eq!(agent.online, agent.target, "hard sync on the interval");
    }

    #[test]
    fn export_import_round_trips_through_serde_json() {
        let mut trained = DqnAgent::new(&[2, 5, 3], 100, 77);
        for i in 0..BATCH_SIZE {
            trained.remember(Experience {
                state: vec![0.2, 0.8],
                action: i % 3,
                reward: 0.5,
                next_state: vec![0.1, 0.1],
                done: i % 4 == 0,
            });
        }
        for _ in 0..5 {
            trained.learn();
        }
        let snap = trained.export_weights();
        let json = serde_json::to_string(&snap).expect("serialize");
        let back: BrainSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, snap);

        let mut fresh = DqnAgent::new(&[2, 5, 3], 100, 1);
        assert_ne!(fresh.export_weights(), snap);
        fresh.import_weights(&back).expect("topology matches");
        assert_eq!(fresh.export_weights(), snap);
        assert_eq!(fresh.epsilon(), trained.epsilon());
        assert_eq!(fresh.step_count(), trained.step_count());
        assert_eq!(fresh.online, trained.online);
        assert_eq!(fresh.online, fresh.target, "import syncs the target too");
        assert_eq!(fresh.q_values(&[0.2, 0.8]), trained.q_values(&[0.2, 0.8]));
    }

    #[test]
    fn a_snapshot_carries_recent_experiences_and_import_refills_the_buffer() {
        let mut a = DqnAgent::new(&[2, 4, 3], 10, 7);
        for i in 0..5 {
            a.remember(Experience {
                state: vec![i as f32, 0.0],
                action: 1,
                reward: 1.0,
                next_state: vec![i as f32, 0.0],
                done: true,
            });
        }
        // One malformed width to be skipped on import.
        let mut snap = a.export_weights();
        assert_eq!(snap.experiences.len(), 5);
        snap.experiences.push(Experience {
            state: vec![9.0],
            action: 0,
            reward: 0.0,
            next_state: vec![9.0],
            done: true,
        });
        let json = serde_json::to_string(&snap).unwrap();
        let back: BrainSnapshot = serde_json::from_str(&json).unwrap();
        let mut b = DqnAgent::new(&[2, 4, 3], 10, 8);
        b.import_weights(&back).unwrap();
        assert_eq!(
            b.buffer_len(),
            5,
            "five well-formed experiences restored, one skipped"
        );
        // An older snapshot without the field still loads.
        let old = r#"{"topology":[2,4,3],"layers":[],"epsilon":0.1,"step_count":0}"#;
        let parsed: Result<BrainSnapshot, _> = serde_json::from_str(old);
        assert!(parsed.is_ok_and(|s| s.experiences.is_empty()));
    }

    #[test]
    fn import_rejects_topology_drift() {
        let donor = DqnAgent::new(&[2, 4, 3], 10, 1);
        let mut agent = DqnAgent::new(&[2, 5, 3], 10, 2);
        let before = agent.export_weights();
        let err = agent
            .import_weights(&donor.export_weights())
            .expect_err("shape drift");
        assert_eq!(
            err,
            ImportError::TopologyMismatch {
                expected: vec![2, 5, 3],
                found: vec![2, 4, 3],
            }
        );
        assert_eq!(
            agent.export_weights(),
            before,
            "a refused import leaves the agent untouched"
        );
        assert!(err.to_string().contains("topology"));
    }

    #[test]
    fn import_rejects_malformed_layer_shapes() {
        let mut agent = DqnAgent::new(&[2, 3, 2], 10, 1);
        let mut snap = agent.export_weights();
        snap.layers[1].biases.pop();
        assert_eq!(
            agent.import_weights(&snap),
            Err(ImportError::LayerShapeMismatch { layer: 1 })
        );
        let mut short = agent.export_weights();
        short.layers.pop();
        assert!(matches!(
            agent.import_weights(&short),
            Err(ImportError::LayerShapeMismatch { .. })
        ));
    }
}
