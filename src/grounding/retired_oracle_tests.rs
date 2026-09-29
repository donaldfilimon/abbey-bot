//! Frozen behavior extracted from the archived Zig oracle, not regenerated.
use super::*;
use serde_json::Value;
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/retired-zig-grounding.json"
    ))
    .unwrap()
}
fn specifics(items: &[Specific]) -> Value {
    serde_json::to_value(
        items
            .iter()
            .map(|s| {
                [
                    format!("{:?}", s.kind).to_ascii_lowercase(),
                    s.text.clone(),
                    s.key.clone(),
                ]
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
#[test]
fn retired_oracle_replays_522_verdicts_and_repairs_six_panics() {
    let data = fixture();
    let (mut checked, mut repaired) = (0, 0);
    for row in data["rows"].as_array().unwrap() {
        let g = Grounding::from_sources(
            row["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap()),
        );
        let reply = row["reply"].as_str().unwrap();
        let verdict = check(reply, &g);
        let hedge = hedged(reply, &verdict);
        if row["panic"] == true {
            repaired += 1;
            continue;
        }
        assert_eq!(specifics(&verdict.examined), row["examined"], "{reply}");
        assert_eq!(specifics(&verdict.ungrounded), row["ungrounded"], "{reply}");
        assert_eq!(
            verdict.grounding_empty,
            row["grounding_empty"].as_bool().unwrap()
        );
        assert_eq!(hedge, row["hedged"].as_str().unwrap(), "{reply}");
        checked += 1;
    }
    assert_eq!((checked, repaired), (522, 6));
}
#[test]
fn utf8_numeric_tails_are_safe_in_both_sources_and_replies() {
    for token in ["12é", "12中", "12𐐀", "12éK", "12中M", "12𐐀T"] {
        let g = Grounding::from_sources([token]);
        assert!(check(token, &g).examined.is_empty(), "{token}");
        assert!(
            check(token, &Grounding::default()).examined.is_empty(),
            "{token}"
        );
    }
    for token in ["12k", "12K", "12m", "12M", "12b", "12B", "12t", "12T"] {
        assert_eq!(check(token, &Grounding::default()).ungrounded.len(), 1);
    }
}
#[test]
fn retired_oracle_replays_192_recall_selections() {
    let data = fixture();
    let rows = data["recall"].as_array().unwrap();
    assert_eq!(rows.len(), 192);
    for row in rows {
        let facts: Vec<String> = serde_json::from_value(
            data["recall_sets"][row["set"].as_u64().unwrap() as usize].clone(),
        )
        .unwrap();
        let selection = crate::recall::select(
            &facts,
            row["query"].as_str().unwrap(),
            row["max"].as_u64().unwrap() as usize,
            row["budget"].as_u64().unwrap() as usize,
        );
        let expected: Vec<&str> = row["chosen"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| facts[i.as_u64().unwrap() as usize].as_str())
            .collect();
        assert_eq!(selection.facts, expected, "{row}");
        assert_eq!(selection.omitted, row["omitted"].as_u64().unwrap() as usize);
    }
}
