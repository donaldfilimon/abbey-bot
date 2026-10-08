//! Explicit offline synthetic evaluation; runs before runtime or state startup.

use std::{ffi::OsString, io::Read, path::PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::brain::quality_evaluation::{
    LABEL_PROVENANCE, QualityCorpus, QualityReport, evaluate_corpus,
};

const MAX_BYTES: u64 = 1024 * 1024;

pub(crate) fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<PathBuf, String> {
    let usage = || "usage: abbey-bot --learning-quality CORPUS.json --json".to_string();
    let path = arguments
        .next()
        .filter(|p| !p.is_empty())
        .ok_or_else(usage)?;
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--json"))
        || arguments.next().is_some()
    {
        return Err(usage());
    }
    Ok(path.into())
}

#[derive(Serialize)]
struct Report {
    schema: u32,
    evaluation: &'static str,
    label_provenance: &'static str,
    action_measurement: &'static str,
    corpus_sha256: String,
    evaluator_source_sha256: String,
    quality: QualityReport,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn evaluate(bytes: &[u8]) -> Result<Report, &'static str> {
    let corpus: QualityCorpus = serde_json::from_slice(bytes).map_err(|_| "invalid_corpus_json")?;
    if corpus.schema != 1 || corpus.label_provenance != LABEL_PROVENANCE {
        return Err("unsupported_corpus_schema_or_provenance");
    }
    let quality = evaluate_corpus(&corpus.cases)?;
    // Domain-separated, length-delimited exact compiled sources. This identity
    // covers the evaluator and its pure dependencies, not the whole checkout.
    let mut identity = Sha256::new();
    identity.update(b"abbey-learning-evaluator-sources-v1");
    for source in [
        include_str!("brain/quality_evaluation.rs"),
        include_str!("brain/dqn.rs"),
        include_str!("brain/nn.rs"),
        include_str!("brain/replay.rs"),
        include_str!("brain/state.rs"),
        include_str!("brain/intent.rs"),
        include_str!("brain/correction.rs"),
        include_str!("grounding.rs"),
        include_str!("runtime.rs"),
        include_str!("learning_quality_cli.rs"),
    ] {
        identity.update((source.len() as u64).to_le_bytes());
        identity.update(source.as_bytes());
    }
    Ok(Report {
        schema: 1,
        evaluation: "synthetic_lexical_support_only",
        label_provenance: LABEL_PROVENANCE,
        action_measurement: "untrained_seeded_policy; [stay, reply, react]; action values are not answer confidence",
        corpus_sha256: digest(bytes),
        evaluator_source_sha256: identity
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        quality,
    })
}

fn read(path: &std::path::Path) -> Result<Vec<u8>, &'static str> {
    read_with_open(path, open_corpus)
}

#[cfg(unix)]
fn open_corpus(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, open};
    open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(std::fs::File::from)
    .map_err(Into::into)
}

#[cfg(not(unix))]
fn open_corpus(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

fn read_with_open(
    path: &std::path::Path,
    open: impl FnOnce(&std::path::Path) -> std::io::Result<std::fs::File>,
) -> Result<Vec<u8>, &'static str> {
    // Validate the object actually opened, not a separately resolved pathname.
    // Unix opens are nonblocking so FIFO substitution cannot stall admission.
    let file = open(path).map_err(|_| "corpus_unreadable")?;
    let metadata = file.metadata().map_err(|_| "corpus_unreadable")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("corpus_requires_regular_file_at_most_1_mib");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "corpus_unreadable")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("corpus_too_large");
    }
    Ok(bytes)
}

pub(crate) fn run(path: &std::path::Path) -> i32 {
    let result = read(path).and_then(|bytes| evaluate(&bytes));
    match result {
        Ok(report) => {
            let passed = report.quality.total.false_positive == 0
                && report.quality.corrections.mismatches == 0;
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{json}"),
                Err(_) => {
                    eprintln!("quality_report_encoding_failed");
                    return 2;
                }
            }
            i32::from(!passed)
        }
        Err(reason) => {
            eprintln!("{reason}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_dispatch_is_explicit_and_rejects_extra_modes() {
        let parse = |args: &[&str]| crate::parse_startup_arguments(args.iter().map(OsString::from));
        assert_eq!(
            parse(&["--learning-quality", "fixture.json", "--json"]).unwrap(),
            crate::StartupAction::LearningQuality(PathBuf::from("fixture.json"))
        );
        for args in [
            vec!["--learning-quality"],
            vec!["--learning-quality", "fixture.json"],
            vec![
                "--learning-quality",
                "fixture.json",
                "--json",
                "--managed-service",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn report_has_counts_and_hashes_without_fixture_content() {
        let bytes = include_bytes!("../tests/fixtures/learning-quality-v1.json");
        let report = evaluate(bytes).unwrap();
        assert_eq!(report.corpus_sha256, digest(bytes));
        assert_eq!(report.evaluator_source_sha256.len(), 64);
        let json = serde_json::to_string(&report).unwrap();
        for text in ["Amber", "Quiet rivers", "source-01", "cited_source_ids"] {
            assert!(!json.contains(text));
        }
        assert_eq!(report.quality.total.false_positive, 0);
    }

    #[test]
    fn corpus_dimensions_are_checked_before_case_evaluation() {
        let mut corpus: serde_json::Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/learning-quality-v1.json"))
                .unwrap();
        corpus["cases"].as_array_mut().unwrap().pop();
        corpus["cases"][0]["id"] = "".into();
        let result = evaluate(&serde_json::to_vec(&corpus).unwrap());
        assert_eq!(result.err(), Some("invalid_corpus_class_counts"));
    }

    #[cfg(unix)]
    #[test]
    fn substituted_fifo_is_refused_from_opened_handle() {
        use rustix::fs::{self, Mode, OFlags};
        let path = std::env::temp_dir().join(format!("abbey-quality-fifo-{}", std::process::id()));
        let result = {
            std::fs::write(&path, "synthetic regular file").unwrap();
            read_with_open(&path, |checked_path| {
                // Deterministically substitute a FIFO at the old check/open
                // boundary. The test opens nonblocking to expose the validation
                // defect without leaving a blocked test owner behind.
                std::fs::remove_file(checked_path)?;
                let created = std::process::Command::new("mkfifo")
                    .arg(checked_path)
                    .status()?;
                assert!(created.success(), "synthetic FIFO creation");
                fs::open(
                    checked_path,
                    OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map(std::fs::File::from)
                .map_err(Into::into)
            })
        };
        std::fs::remove_file(&path).unwrap();
        assert_eq!(result, Err("corpus_requires_regular_file_at_most_1_mib"));
    }

    #[cfg(unix)]
    #[test]
    fn production_open_sets_nonblocking_and_reads_regular_files() {
        use rustix::fs::{OFlags, fcntl_getfl};
        let path =
            std::env::temp_dir().join(format!("abbey-quality-regular-{}", std::process::id()));
        std::fs::write(&path, b"synthetic").unwrap();
        let file = open_corpus(&path).unwrap();
        let flags = fcntl_getfl(&file).unwrap();
        let bytes = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(flags.contains(OFlags::NONBLOCK));
        assert_eq!(bytes, Ok(b"synthetic".to_vec()));
    }
}
