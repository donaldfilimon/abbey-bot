//! A presentation input that cannot carry process-wide activity aggregates.
pub struct ScopedStatsInput<'a> {
    /// Fixed shell-selected label: "This server" or "Your DM".
    pub scope_label: &'a str,
    /// Computed exclusively from the current scope's brain registry entry.
    pub brain_summary: &'a str,
    pub budget_per_hour: u32,
    pub tokens_left: f32,
}

pub fn render_scoped_stats(input: &ScopedStatsInput<'_>) -> String {
    format!(
        "{} · learning and reply budget\n{}\nReply budget: {:.1} of {}/h left",
        input.scope_label, input.brain_summary, input.tokens_left, input.budget_per_hour,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_readable_scoped_budget_and_brain() {
        let rendered = render_scoped_stats(&ScopedStatsInput {
            scope_label: "Your DM",
            brain_summary: "Brain: not loaded for this conversation yet",
            budget_per_hour: 6,
            tokens_left: 4.5,
        });
        assert_eq!(
            rendered,
            "Your DM · learning and reply budget\nBrain: not loaded for this conversation yet\nReply budget: 4.5 of 6/h left"
        );
    }
}
