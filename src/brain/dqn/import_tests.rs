//! Regression coverage for complete, atomic snapshot replacement.
use super::*;
use crate::brain::registry::Brain;

fn experience(tag: u16) -> Experience {
    Experience {
        state: vec![0.25, 0.75],
        action: usize::from(tag) % 3,
        reward: f32::from(tag) / 4096.0,
        next_state: vec![0.5, 0.5],
        done: false,
    }
}

fn used_agent(capacity: usize) -> DqnAgent {
    let mut agent = DqnAgent::new(&[2, 4, 3], capacity, 73);
    for i in 0..u16::try_from(capacity + 7).unwrap() {
        agent.remember(experience(i));
    }
    agent.learn();
    assert_ne!(agent.online, agent.target, "fixture has a lagging target");
    agent
}

fn assert_unchanged(actual: &DqnAgent, before: &DqnAgent) {
    assert_eq!(actual.online, before.online);
    assert_eq!(actual.target, before.target);
    assert_eq!(
        actual.buffer, before.buffer,
        "includes uncapped replay/cursor/capacity"
    );
    assert_eq!(actual.epsilon, before.epsilon);
    assert_eq!(actual.step_count, before.step_count);
    assert_eq!(actual.rng, before.rng);
}

#[test]
fn invalid_actions_refuse_used_agent_atomically() {
    let mut agent = used_agent(SNAPSHOT_EXPERIENCES + 17);
    let before = agent.clone();
    for action in [agent.action_count(), usize::MAX] {
        for invalid_width in [false, true] {
            let mut snapshot = before.export_weights();
            snapshot.experiences[7].action = action;
            if invalid_width {
                snapshot.experiences[7].state.pop();
            }
            assert_eq!(
                agent.import_weights(&snapshot),
                Err(ImportError::ReplayActionOutOfRange {
                    experience: 7,
                    action,
                    action_count: 3,
                })
            );
            assert_unchanged(&agent, &before);
        }
    }
}

#[test]
fn json_invalid_action_cannot_restore_a_panicking_learning_batch() {
    let mut agent = used_agent(32);
    let before = agent.clone();
    let mut snapshot = before.export_weights();
    snapshot.experiences = vec![experience(1); 8];
    for exp in &mut snapshot.experiences {
        exp.action = agent.action_count();
    }
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(!agent.import_json(&json));
    assert_unchanged(&agent, &before);
}

#[test]
fn valid_restore_replaces_used_replay_and_is_repeatable() {
    let mut agent = used_agent(32);
    let capacity = agent.buffer.capacity();
    let rng = agent.rng.clone();
    let mut donor = DqnAgent::new(&[2, 4, 3], 32, 11);
    donor.remember(experience(90));
    donor.remember(experience(91));
    let snapshot = donor.export_weights();
    for _ in 0..2 {
        agent.import_weights(&snapshot).unwrap();
        assert_eq!(agent.export_weights(), snapshot);
        assert_eq!(agent.buffer.capacity(), capacity);
        assert_eq!(agent.target, agent.online);
        assert_eq!(agent.rng, rng, "snapshot has no RNG state");
    }
}

#[test]
fn legacy_empty_replay_clears_a_used_agent() {
    let mut agent = used_agent(32);
    let snapshot = DqnAgent::new(&[2, 4, 3], 32, 11).export_weights();
    let mut json = serde_json::to_value(&snapshot).unwrap();
    json.as_object_mut().unwrap().remove("experiences");
    assert!(agent.import_json(&json.to_string()));
    assert_eq!(agent.export_weights(), snapshot);
    assert_eq!(agent.buffer.capacity(), 32);
}

#[test]
fn restore_retains_newest_compatible_rows_with_destination_capacity() {
    let mut agent = DqnAgent::new(&[2, 4, 3], 3, 73);
    for tag in 0..7 {
        agent.remember(experience(tag));
    }
    let mut snapshot = agent.export_weights();
    snapshot.experiences = (90..96).map(experience).collect();
    let mut legacy = experience(99);
    legacy.next_state.pop();
    snapshot.experiences.insert(4, legacy);
    agent.import_weights(&snapshot).unwrap();
    assert_eq!(agent.buffer.capacity(), 3);
    assert_eq!(
        agent.export_weights().experiences,
        (93..96).map(experience).collect::<Vec<_>>()
    );
    agent.remember(experience(96));
    assert_eq!(
        agent.export_weights().experiences,
        (94..97).map(experience).collect::<Vec<_>>()
    );
}

#[test]
fn refused_snapshots_preserve_all_trained_state_beyond_exported_replay_tail() {
    let mut agent = used_agent(SNAPSHOT_EXPERIENCES + 17);
    let mut before = agent.clone();
    for field in 0..7 {
        let mut invalid = before.export_weights();
        match field {
            0 => invalid.topology[0] += 1,
            1 => {
                invalid.layers.last_mut().unwrap().biases.pop();
            }
            2 => invalid.epsilon = f32::NAN,
            3 => invalid.layers.last_mut().unwrap().weights[0] = f32::INFINITY,
            4 => invalid.experiences[7].reward = f32::NEG_INFINITY,
            5 => invalid.experiences[7].state[0] = f32::NAN,
            6 => invalid.experiences[7].next_state[0] = f32::INFINITY,
            _ => unreachable!(),
        }
        assert!(agent.import_weights(&invalid).is_err(), "field {field}");
        assert_unchanged(&agent, &before);
    }
    agent.learn();
    before.learn();
    assert_unchanged(&agent, &before);
}
