use super::*;
use crate::platform::{EventKind, SocialNetwork};

#[tokio::test]
async fn blocked_channel_welcome_stops_before_backend_or_outbound() {
    let state = AppState::in_memory();
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.guilds).update("discord:g", &mut *stores, |settings| {
            settings.unsolicited = true;
            settings.learning_enabled = false;
            settings.unsolicited_channels = Some(Default::default());
        });
    }
    let out = super::testing::FakeOut::default();
    let event = SocialEvent {
        network: SocialNetwork::Discord,
        kind: EventKind::MemberJoined,
        native_message_id: "m".into(),
        native_channel_id: "c".into(),
        native_guild_id: Some("g".into()),
        native_user_id: "u".into(),
        user_display_name: "Member".into(),
        is_bot: false,
        timestamp: 0,
    };
    assert_eq!(
        handle(&state, &out, event, false, None).await,
        Outcome::Ignored("channel participation off")
    );
    assert!(out.sent.lock().unwrap().is_empty());
}

#[test]
fn learning_off_does_not_block_participation_but_opt_in_and_quiet_do() {
    let state = AppState::in_memory();
    let mut ctx = Ctx {
        event: SocialEvent {
            network: SocialNetwork::Discord,
            kind: EventKind::Message {
                text: "hello".into(),
                attachments: vec![],
            },
            native_message_id: "m".into(),
            native_channel_id: "c".into(),
            native_guild_id: Some("g".into()),
            native_user_id: "u".into(),
            user_display_name: "Member".into(),
            is_bot: false,
            timestamp: 0,
        },
        text: "hello".into(),
        attachments: vec![],
        forced: false,
        settings: GuildSettings {
            unsolicited: true,
            learning_enabled: false,
            ..GuildSettings::default()
        },
        heat: 0,
        scoped_user: "discord:u".into(),
        scoped_guild: "discord:g".into(),
        scoped_channel: "discord:c".into(),
    };
    assert_eq!(guards(&ctx, &state), Ok(()));
    ctx.settings.unsolicited_channels = Some(Default::default());
    assert_eq!(
        guards(&ctx, &state),
        Err(Outcome::Ignored("channel participation off"))
    );
    ctx.forced = true;
    assert_eq!(guards(&ctx, &state), Ok(()));
    ctx.forced = false;
    ctx.settings
        .unsolicited_channels
        .as_mut()
        .unwrap()
        .insert(ctx.scoped_channel.clone());
    assert_eq!(guards(&ctx, &state), Ok(()));
    ctx.settings.unsolicited = false;
    assert_eq!(guards(&ctx, &state), Err(Outcome::Ignored("act off")));
    ctx.forced = true;
    assert_eq!(guards(&ctx, &state), Ok(()));
    assert_eq!(
        check_unsolicited(&ctx.settings, true, false),
        Err(Outcome::Ignored("quiet"))
    );
}
