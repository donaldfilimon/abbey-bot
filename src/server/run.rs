//! `abbey-bot --server-plan`: the operator entry point for the plan engine.
//!
//! Dry run by default. `--apply` performs the changes the dry run listed, then
//! re-reads the guild and diffs again, so the exit code says whether the guild
//! now matches what was printed rather than whether the requests returned 200.
//!
//! Exit codes follow the other CLI modes: 0 done (or nothing to do), 1 the
//! guild refused or a blocker stands, 2 the invocation or plan is wrong.

use std::path::PathBuf;

use serenity::http::Http;
use serenity::model::id::GuildId;

use super::apply::apply;
use super::diff::{Report, Scope, Stage, diff};
use super::discord::{DiscordWriter, permission_bits, snapshot};
use super::plan::Plan;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub plan: PathBuf,
    pub guild_id: u64,
    pub stage: Stage,
    pub category: Option<String>,
    pub apply: bool,
}

pub const USAGE: &str = "usage: abbey-bot --server-plan PLAN.toml --guild ID [--stage additive|reveal|overwrites] [--category NAME] [--apply]";

fn verify_applied(applied: usize, stage: Stage, remaining: &Report) -> Result<String, String> {
    if remaining.is_clear() && remaining.changes.is_empty() {
        return Ok(format!(
            "verified: {applied} change(s) applied; the guild now matches stage {} of the plan.",
            stage.label()
        ));
    }
    let mut message = format!(
        "applied {applied} change(s), but post-apply verification failed:\n{}",
        remaining.render("POST-APPLY VERIFICATION")
    );
    message.push_str("Resolve the blockers or remaining changes above, then re-run the dry run.\n");
    Err(message)
}

/// Parse the arguments after `--server-plan`.
pub fn parse_options(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<Options, String> {
    let plan = arguments
        .next()
        .filter(|p| !p.is_empty() && !p.to_string_lossy().starts_with("--"))
        .ok_or_else(|| format!("{USAGE} (the plan path comes first)"))?;
    let mut guild_id = None;
    let mut stage = Stage::Additive;
    let mut category = None;
    let mut apply = false;
    while let Some(flag) = arguments.next() {
        match flag.to_str() {
            Some("--guild") => {
                let raw = arguments
                    .next()
                    .ok_or_else(|| format!("{USAGE} (--guild needs a snowflake)"))?;
                let parsed = raw
                    .to_str()
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .filter(|id| *id != 0)
                    .ok_or_else(|| {
                        format!("{USAGE} (--guild must be a nonzero numeric snowflake)")
                    })?;
                guild_id = Some(parsed);
            }
            Some("--stage") => {
                let raw = arguments
                    .next()
                    .ok_or_else(|| format!("{USAGE} (--stage needs a name)"))?;
                stage = raw.to_str().and_then(Stage::parse).ok_or_else(|| {
                    format!("{USAGE} (--stage must be additive, reveal, or overwrites)")
                })?;
            }
            Some("--category") => {
                let raw = arguments
                    .next()
                    .ok_or_else(|| format!("{USAGE} (--category needs a name)"))?;
                let name = raw
                    .to_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("{USAGE} (--category needs a name)"))?;
                category = Some(name.to_string());
            }
            Some("--apply") => apply = true,
            _ => return Err(format!("{USAGE} (unknown argument {flag:?})")),
        }
    }
    let guild_id = guild_id.ok_or_else(|| format!("{USAGE} (--guild is required)"))?;
    if stage == Stage::Overwrites && category.is_none() {
        return Err(format!(
            "{USAGE} (--stage overwrites needs --category NAME)"
        ));
    }
    if stage != Stage::Overwrites && category.is_some() {
        return Err(format!(
            "{USAGE} (--category only applies to --stage overwrites)"
        ));
    }
    Ok(Options {
        plan: plan.into(),
        guild_id,
        stage,
        category,
        apply,
    })
}

/// Load and validate the plan file, including its permission vocabulary.
pub fn load_plan(path: &std::path::Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let plan = Plan::from_toml(&text)?;
    let names: Vec<String> = plan
        .permission_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    permission_bits(&names).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(plan)
}

/// Run the mode. Prints to stdout; returns the process exit code.
pub async fn run(options: &Options, http: &Http) -> i32 {
    let plan = match load_plan(&options.plan) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let guild_id = GuildId::new(options.guild_id);
    let scope = Scope {
        stage: options.stage,
        category: options.category.clone(),
    };
    let heading = |mode: &str| {
        format!(
            "plan {:?} from {} · guild {} · stage {}{} · {mode}",
            plan.name,
            options.plan.display(),
            options.guild_id,
            options.stage.label(),
            options
                .category
                .as_deref()
                .map(|c| format!(" · category {c:?}"))
                .unwrap_or_default()
        )
    };

    let before = match snapshot(http, guild_id).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let report = diff(&plan, &before, &scope);
    println!(
        "{}",
        report.render(&heading(if options.apply { "APPLY" } else { "DRY RUN" }))
    );
    if !report.is_clear() {
        println!("Nothing was applied: resolve the blockers above and re-run.");
        return 1;
    }
    if report.changes.is_empty() {
        println!("Nothing to do: the guild already matches this stage of the plan.");
        return 0;
    }
    if !options.apply {
        println!(
            "Nothing was applied. Re-run with --apply to perform these {} change(s).",
            report.changes.len()
        );
        return 0;
    }

    let mut writer = DiscordWriter {
        http,
        guild_id,
        reason: format!(
            "abbey-bot --server-plan {} --stage {}",
            plan.name,
            options.stage.label()
        ),
    };
    println!("applying {} change(s)…", report.changes.len());
    let outcome = apply(&report.changes, &before, &mut writer).await;
    print!("{}", outcome.render());
    if outcome.failed.is_some() {
        return 1;
    }

    // The proof: read the guild back and confirm this stage has nothing left.
    match snapshot(http, guild_id).await {
        Ok(after) => {
            let remaining = diff(&plan, &after, &scope);
            match verify_applied(outcome.applied.len(), options.stage, &remaining) {
                Ok(message) => {
                    println!("{message}");
                    0
                }
                Err(message) => {
                    print!("{message}");
                    1
                }
            }
        }
        Err(error) => {
            println!(
                "applied {} change(s), but the verification read failed: {error}",
                outcome.applied.len()
            );
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Options, String> {
        parse_options(arguments.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn dry_run_is_the_default_and_guild_is_required() {
        let options = parse(&["plan.toml", "--guild", "42"]).unwrap();
        assert_eq!(
            options,
            Options {
                plan: PathBuf::from("plan.toml"),
                guild_id: 42,
                stage: Stage::Additive,
                category: None,
                apply: false,
            }
        );
        assert!(
            parse(&["plan.toml"])
                .unwrap_err()
                .contains("--guild is required")
        );
        assert!(parse(&[]).unwrap_err().contains("plan path"));
        assert!(parse(&["--guild", "42"]).unwrap_err().contains("plan path"));
    }

    #[test]
    fn apply_and_stages_parse_and_category_is_tied_to_overwrites() {
        let options = parse(&[
            "p.toml",
            "--guild",
            "42",
            "--stage",
            "overwrites",
            "--category",
            "STAFF",
            "--apply",
        ])
        .unwrap();
        assert_eq!(options.stage, Stage::Overwrites);
        assert_eq!(options.category.as_deref(), Some("STAFF"));
        assert!(options.apply);
        assert_eq!(
            parse(&["p.toml", "--guild", "42", "--stage", "reveal"])
                .unwrap()
                .stage,
            Stage::Reveal
        );
        assert!(
            parse(&["p.toml", "--guild", "42", "--stage", "overwrites"])
                .unwrap_err()
                .contains("needs --category")
        );
        assert!(
            parse(&["p.toml", "--guild", "42", "--category", "STAFF"])
                .unwrap_err()
                .contains("only applies")
        );
        assert!(
            parse(&["p.toml", "--guild", "42", "--stage", "delete"])
                .unwrap_err()
                .contains("must be additive")
        );
    }

    #[test]
    fn a_zero_or_non_numeric_guild_cannot_reach_guild_id_new() {
        for bad in ["0", "abc", "", "12x"] {
            let error = parse(&["p.toml", "--guild", bad]).unwrap_err();
            assert!(
                error.contains("nonzero numeric snowflake"),
                "{bad:?}: {error}"
            );
        }
        assert!(
            parse(&["p.toml", "--guild", "42", "--bogus"])
                .unwrap_err()
                .contains("unknown argument")
        );
        assert!(
            parse(&["p.toml", "--guild", "42", "--category"])
                .unwrap_err()
                .contains("needs a name")
        );
    }

    #[test]
    fn the_shipped_plan_loads_through_the_same_path_the_cli_uses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("blueprints")
            .join("mlai-community.toml");
        let plan = load_plan(&path).expect("shipped plan loads");
        assert_eq!(plan.name, "MLAI");
        let missing = load_plan(&path.with_file_name("does-not-exist.toml")).unwrap_err();
        assert!(missing.contains("cannot read"), "{missing}");
    }

    #[test]
    fn post_apply_verification_fails_closed_and_renders_new_blockers() {
        use crate::server::observe::{
            BotState, ChannelClass, ChannelState, GuildSnapshot, RoleState,
        };

        let plan = Plan::from_toml(
            r#"
name = "verification"
[[roles]]
name = "Abbey"
[[categories]]
name = "Main"
[[categories.channels]]
name = "general"
kind = "text"
"#,
        )
        .unwrap();
        let role = |id, name: &str, position, managed, permissions: &[&str]| RoleState {
            id,
            name: name.into(),
            position,
            managed,
            hoist: false,
            mentionable: false,
            colour: 0,
            permissions: permissions.iter().map(|name| (*name).into()).collect(),
        };
        let snapshot = GuildSnapshot {
            guild_id: 42,
            features: Vec::new(),
            roles: vec![
                role(42, "@everyone", 0, false, &["View Channel"]),
                role(50, "Abbey Bot", 3, true, &["Administrator"]),
                role(51, "Abbey", 2, true, &[]),
            ],
            channels: vec![
                ChannelState {
                    id: 60,
                    name: "Main".into(),
                    class: ChannelClass::Category,
                    parent: None,
                    topic: None,
                    overwrites: Vec::new(),
                },
                ChannelState {
                    id: 61,
                    name: "general".into(),
                    class: ChannelClass::Kind(crate::server::ChannelKind::Text),
                    parent: Some(60),
                    topic: None,
                    overwrites: Vec::new(),
                },
            ],
            bot: BotState {
                user_id: 7,
                role_ids: vec![50],
            },
        };
        let remaining = diff(
            &plan,
            &snapshot,
            &Scope {
                stage: Stage::Reveal,
                category: None,
            },
        );
        assert!(remaining.changes.is_empty(), "{:?}", remaining.changes);
        assert!(!remaining.blockers.is_empty());
        let error = verify_applied(3, Stage::Reveal, &remaining).unwrap_err();
        assert!(error.contains("post-apply verification failed"), "{error}");
        assert!(error.contains("integration-managed role"), "{error}");
        assert!(error.contains("Resolve the blockers"), "{error}");
    }
}
