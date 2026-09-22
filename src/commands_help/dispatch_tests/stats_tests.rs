//! Registered `/stats`: guild scoping of brain and budget output.
use super::*;

async fn stats_output(fixture: &DiscordFixture, data: &Data, in_guild: bool) -> String {
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::Stats);
    let invocation = Invocation::new(command, in_guild, None);
    let options = poise::FrameworkOptions::default();
    let context = invocation.context(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
    );
    assert!(command.slash_action.unwrap()(context).await.is_ok());
    let requests = fixture.take_requests();
    assert_private_no_mentions_reply(&requests).to_string()
}

#[tokio::test]
async fn registered_stats_ignores_other_guilds_and_dms_but_keeps_own_brain_and_budget() {
    let fixture = DiscordFixture::new().await;
    let data = configured_data();
    let guild_before = stats_output(&fixture, &data, true).await;
    let dm_before = stats_output(&fixture, &data, false).await;
    assert!(guild_before.starts_with("This server"));
    assert!(dm_before.starts_with("Your DM"));
    {
        let mut stores = runtime::AppState::lock(&data.state.stores);
        stores.memory.messages_seen += 900;
        stores
            .memory
            .interactions
            .record(crate::memory::InteractionEntry::new(
                "stats", true, None, 1, 1000,
            ));
        for scope in ["discord:other-guild", "discord:dm:790", "discord:dm:791"] {
            stores
                .memory
                .record_message(scope, "unrelated-user", "private unrelated activity", 1);
            runtime::AppState::lock(&data.state.rewards).register_reply(
                vec![0.5],
                1,
                scope,
                scope,
                1,
            );
            runtime::AppState::lock(&data.state.brains)
                .brain(scope, &*stores, runtime::now())
                .set_epsilon(0.8);
            runtime::AppState::lock(&data.state.budget).try_take(scope, 6, runtime::now());
        }
    }
    assert_eq!(stats_output(&fixture, &data, true).await, guild_before);
    assert_eq!(stats_output(&fixture, &data, false).await, dm_before);
    {
        let stores = runtime::AppState::lock(&data.state.stores);
        runtime::AppState::lock(&data.state.brains)
            .brain(&format!("discord:{GUILD}"), &*stores, runtime::now())
            .set_epsilon(0.123);
        assert!(runtime::AppState::lock(&data.state.budget).try_take(
            &format!("discord:{GUILD}"),
            6,
            runtime::now()
        ));
    }
    let changed = stats_output(&fixture, &data, true).await;
    assert_ne!(changed, guild_before);
    assert!(changed.contains("0.123"));
    assert!(changed.contains("5.0 of 6/h"));
    assert_eq!(stats_output(&fixture, &data, false).await, dm_before);
}
