//! Live acceptance of the memory path: the real `abi` binary against a real
//! `abi-wdbx-gateway`, through the same code the bot runs (tool host, queue,
//! drain, slash-style admit and forget, gated checkpoint persist), with every
//! receipt confirmed by `abi wdbx episode verify` from a separate process.
//!
//! Ignored by default, so an unset environment can never turn it into a
//! silent green. Run it on purpose:
//!
//! ```text
//! ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG="$HOME/.config/abbey-bot/episode-gate-acceptance.json" \
//!   cargo test acceptance -- --ignored --nocapture
//! ```
//!
//! The config is an ordinary gate config whose `guilds` list covers exactly
//! two scopes: [`SCOPE`], which the gateway policy must key as
//! `discord-acceptance`, and [`UNLISTED_SCOPE`], which the policy must not
//! list at all (it proves the fail-closed refusal live). Point it at a scratch
//! gateway, not the production store: each run appends four events and
//! charges their bytes against the acceptance guild's budget forever.
//!
//! What this does not claim: no Discord traffic, no second bot instance on
//! the live token. It is the bot's memory code path end to end, not a slash
//! command.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use super::{EpisodeGate, EpisodeGateConfig};
use crate::checkpoint_gate;
use crate::memory_gate::{self, Drained};
use crate::persist::BrainRow;
use crate::platform::SocialNetwork;
use crate::runtime::{AppState, RememberOutcome, ToolScope, now, now_millis};
use crate::tools::ToolHost as _;

pub const ACCEPTANCE_ENV: &str = "ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG";
/// Keyed as `discord-acceptance` in the gateway policy.
pub const SCOPE: &str = "discord:acceptance";
/// Covered by the bot config, absent from the gateway policy.
pub const UNLISTED_SCOPE: &str = "discord:acceptance-unlisted";
const USER: &str = "discord:42";

/// `abi wdbx episode verify` as a separate process, the way an operator
/// would check a receipt. Returns the parsed JSON object on exit 0.
fn verify(config: &EpisodeGateConfig, guild_ref: &str, digest_hex: &str) -> serde_json::Value {
    let mut command = Command::new(&config.abi_cli);
    command
        .env_clear()
        .args([
            "wdbx",
            "episode",
            "verify",
            guild_ref,
            digest_hex,
            "--endpoint",
            &config.endpoint,
            "--token-file",
        ])
        .arg(&config.token_file)
        .arg("--json");
    for key in super::ALLOWED_ENVIRONMENT {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if let Some(ca_cert) = &config.ca_cert {
        command.arg("--ca-cert").arg(ca_cert);
    }
    let output = command.output().expect("abi verify must start");
    assert!(
        output.status.success(),
        "verify {guild_ref} {digest_hex}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("verify --json prints one object")
}

fn assert_found(receipt: &serde_json::Value, event_kind: &str, guild_ref: &str) {
    assert_eq!(receipt["found"], "true", "{receipt}");
    assert_eq!(receipt["event_kind"], event_kind, "{receipt}");
    assert_eq!(receipt["guild_ref"], guild_ref, "{receipt}");
    assert_eq!(receipt["terminal_status"], "completed", "{receipt}");
}

/// Operator preflight for a config about to go live: the exact validation
/// `AppState::from_env` runs at startup, where a failure is a `StartupError`
/// and, under launchd `KeepAlive`, a crash loop. The bot itself cannot be
/// used for this check because Discord authentication precedes it.
///
/// ```text
/// ABBEY_EPISODE_GATE_CONFIG="$HOME/.config/abbey-bot/episode-gate.json" \
///   cargo test preflight -- --ignored
/// ```
#[test]
#[ignore = "preflight: needs ABBEY_EPISODE_GATE_CONFIG naming the config about to go live"]
fn preflight_the_gate_config_named_by_the_environment() {
    let config = EpisodeGateConfig::from_env()
        .expect("the config must validate exactly as at startup")
        .unwrap_or_else(|| panic!("{} is unset or blank", super::CONFIG_ENV));
    assert!(
        config.abi_cli.is_file() && config.token_file.is_file(),
        "abi_cli and token_file must exist"
    );
    eprintln!(
        "preflight ok: endpoint {} timeout {}s coverage {:?}",
        config.endpoint(),
        config.timeout_secs(),
        config.coverage()
    );
}

#[tokio::test]
#[ignore = "live acceptance: needs a running gateway and ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG"]
async fn live_memory_path_round_trips_through_a_real_gateway() {
    let path = std::env::var(ACCEPTANCE_ENV)
        .unwrap_or_else(|_| panic!("{ACCEPTANCE_ENV} must name the acceptance gate config"));
    let config = EpisodeGateConfig::from_path(Path::new(&path)).expect("acceptance config");
    assert_eq!(
        config.coverage(),
        Some(2),
        "the acceptance config must cover exactly {SCOPE} and {UNLISTED_SCOPE}"
    );
    assert!(config.covers(SCOPE) && config.covers(UNLISTED_SCOPE));
    let gate = Arc::new(EpisodeGate::new(config.clone()));
    let mut state = AppState::in_memory();
    Arc::get_mut(&mut state).unwrap().episode_gate = Some(gate.clone());
    let (g, u) = (SCOPE, USER);
    let guild_ref = super::guild_ref_for(g).unwrap();
    let run = now_millis();

    // 1. The model's memory tool queues; the drain proposes, the gateway
    //    appends, and only then is the fact stored, keyed by its receipt.
    let fact = format!("acceptance run {run} likes compilers");
    let mut host = ToolScope {
        memory_turn: None,
        state: &state,
        network: SocialNetwork::Discord,
        scoped_guild: g.into(),
        scoped_user: u.into(),
        scoped_channel: "discord:acceptance-channel".into(),
        now: now(),
        persona: crate::persona::Persona::Abbey,
    };
    let reply = host.remember_fact(&fact, None);
    assert!(reply.starts_with("Queued"), "{reply}");
    assert!(state.memory_service().facts(g, u).is_empty());
    assert_eq!(
        memory_gate::drain(&state).await,
        Drained {
            admitted: 1,
            refused: 0
        }
    );
    assert_eq!(state.memory_service().facts(g, u), vec![fact.clone()]);
    let first = state
        .memory_service()
        .receipt(g, u, &fact)
        .expect("an admitted fact carries its receipt");
    assert_found(
        &verify(&config, &guild_ref, &first),
        "memory_candidate",
        &guild_ref,
    );

    // 2. The slash path: `/remember --replaces` supersedes the first receipt.
    let replacement = format!("acceptance run {run} likes linkers");
    let second = memory_gate::admit_fact(&state, g, u, &replacement, Some(&fact))
        .await
        .expect("admitted")
        .expect("gated scope yields a receipt");
    let outcome = state
        .memory_service()
        .remember_replacing(g, u, &replacement, &fact, now())
        .expect("local store accepts the admitted write");
    let RememberOutcome::Superseded { stored, removed } = outcome else {
        panic!("expected a supersession, got {outcome:?}");
    };
    memory_gate::settle_receipts(&state, g, u, &stored, Some(&removed), &second);
    assert_eq!(
        state.memory_service().facts(g, u),
        vec![replacement.clone()]
    );
    assert!(state.memory_service().receipt(g, u, &fact).is_none());
    assert_found(
        &verify(&config, &guild_ref, &second),
        "memory_candidate",
        &guild_ref,
    );

    // 3. `/forget`: a tombstone is appended before the local delete.
    memory_gate::admit_forget(&state, g, u, &replacement)
        .await
        .expect("tombstone admitted");
    assert!(state.memory_service().forget(g, u, &replacement));
    memory_gate::drop_receipt(&state, g, u, &replacement);
    assert!(state.memory_service().facts(g, u).is_empty());
    assert!(state.memory_service().receipt(g, u, &replacement).is_none());

    // 4. A changed brain checkpoint is proposed by the gated persist and its
    //    receipt recorded; the persist itself is memory-only here.
    let row = BrainRow {
        snapshot_json: format!("{{\"acceptance\":{run}}}"),
        experience_count: 1,
    };
    AppState::lock(&state.stores)
        .brains
        .insert(g.to_owned(), row.clone());
    state.persist_all_gated().await;
    let checkpoint = AppState::lock(&state.checkpoints)
        .get(g)
        .cloned()
        .expect("the checkpoint was admitted");
    assert_eq!(checkpoint.row, row);
    assert_eq!(
        checkpoint.commitment,
        checkpoint_gate::commitment(&checkpoint_gate::payload(&row))
    );
    let digest = checkpoint
        .episode_digest
        .map(|bytes| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .expect("an admitted checkpoint carries its digest");
    assert_found(
        &verify(&config, &guild_ref, &digest),
        "memory_candidate",
        &guild_ref,
    );

    // 5. Fail closed, live: a covered scope the gateway policy does not list
    //    is refused with the store's reason, and nothing is stored.
    let error = memory_gate::admit_fact(&state, UNLISTED_SCOPE, u, "never stored", None)
        .await
        .expect_err("the policy does not know this guild");
    assert!(
        error.starts_with("Not stored: the constitutional memory gate refused it"),
        "{error}"
    );
    assert!(state.memory_service().facts(UNLISTED_SCOPE, u).is_empty());

    let counters = gate.counters();
    assert_eq!(
        (
            counters.appended,
            counters.rejected,
            counters.unavailable,
            counters.ungated_forgets
        ),
        (4, 1, 0, 0),
        "{counters:?}"
    );
    eprintln!(
        "acceptance ok: run {run} guild {guild_ref} receipts {first} {second} {digest} counters {counters:?}"
    );
}
