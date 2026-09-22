//! Registered `/voice` commands: join/resume acknowledgement, leave teardown ordering, status copy.
use super::*;

async fn invoke_voice_slash_fails(
    fixture: &DiscordFixture,
    command: &poise::Command<Data, Error>,
    data: &Data,
    consent: Option<bool>,
) -> bool {
    let invocation = Invocation::voice(command, consent);
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        fixture,
        command,
        &options,
        data,
        poise::CommandInteractionType::Command,
        &args,
    );
    command.slash_action.unwrap()(context).await.is_err()
}

#[tokio::test]
async fn registered_voice_join_and_resume_stop_when_acknowledgement_fails() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture.fail_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let commands = crate::application_commands();
    for key in [CommandKey::VoiceJoin, CommandKey::VoiceResume] {
        let command = command_by_key(&commands, key);
        assert!(invoke_voice_slash_fails(&fixture, command, &data, Some(true)).await);
        let requests = fixture.take_requests();
        assert_deferred_first(&requests, command);
        assert_eq!(requests.len(), 1, "{}", command.qualified_name);
    }
}

#[tokio::test]
async fn registered_voice_join_and_resume_do_not_mutate_lifecycle_while_acknowledgement_waits() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let data = configured_data();
    let runtime = data.voice.as_ref().unwrap();
    let commands = crate::application_commands();

    for key in [CommandKey::VoiceJoin, CommandKey::VoiceResume] {
        let command = command_by_key(&commands, key);
        let invocation = Invocation::voice(command, Some(true));
        let args = invocation.interaction.data.options();
        let options = poise::FrameworkOptions::default();
        let context = invocation.context_with_args(
            &fixture,
            command,
            &options,
            &data,
            poise::CommandInteractionType::Command,
            &args,
        );
        let before = runtime.snapshot().await;
        let action = command.slash_action.unwrap()(context);
        tokio::pin!(action);
        tokio::select! {
            permit = fixture.acknowledgement_entered.acquire() => permit.unwrap().forget(),
            _ = &mut action => panic!("{} completed before held acknowledgement", command.qualified_name),
        }
        let waiting = runtime.snapshot().await;
        assert_eq!(waiting.epoch, before.epoch, "{}", command.qualified_name);
        assert_eq!(waiting.phase, before.phase, "{}", command.qualified_name);
        assert_eq!(
            waiting.media_enabled, before.media_enabled,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.start_pending, before.start_pending,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.consent_epoch, before.consent_epoch,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            waiting.participant_count, before.participant_count,
            "{}",
            command.qualified_name
        );
        assert_eq!(
            fixture.take_requests().len(),
            1,
            "{}",
            command.qualified_name
        );
        fixture.acknowledgement_release.add_permits(1);
        let _ = action.await;
        fixture.take_requests();
    }
}

#[tokio::test]
async fn registered_authorized_voice_leave_closes_pending_media_before_teardown_awaits() {
    let fixture = DiscordFixture::new().await;
    fixture.actor_presence(true);
    let data = configured_data();
    let runtime = data.voice.as_ref().unwrap();
    runtime.reserve_start();
    assert!(runtime.snapshot().await.start_pending);
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceLeave);

    fixture.hold_acknowledgement.store(true, Ordering::SeqCst);
    let transition = Arc::new(crate::commands_voice::VoiceLeaveTransitionProbe::new());
    fixture
        .context
        .data
        .write()
        .await
        .insert::<crate::commands_voice::VoiceLeaveTransitionProbeKey>(Arc::clone(&transition));
    let invocation = Invocation::voice(command, None);
    let args = invocation.interaction.data.options();
    let options = poise::FrameworkOptions::default();
    let context = invocation.context_with_args(
        &fixture,
        command,
        &options,
        &data,
        poise::CommandInteractionType::Command,
        &args,
    );
    let action = command.slash_action.unwrap()(context);
    tokio::pin!(action);
    let (acknowledgement, teardown) = tokio::select! {
        entered = async {
            tokio::join!(
                fixture.acknowledgement_entered.acquire(),
                transition.entered.acquire()
            )
        } => entered,
        _ = &mut action => panic!("voice leave completed before held branches entered"),
    };
    acknowledgement.unwrap().forget();
    teardown.unwrap().forget();

    // Both awaited branches have started and remain independently blocked.
    // The real adapter must already have performed the synchronous close.
    assert!(!runtime.snapshot().await.start_pending);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert_eq!(requests.len(), 1);
    fixture.acknowledgement_release.add_permits(1);
    transition.release.add_permits(1);
    assert!(action.await.is_err());
}

#[tokio::test]
async fn actual_member_voice_status_hides_channel_and_runs_no_provider_probe() {
    let fixture = DiscordFixture::new().await;
    let providers = ProviderFixture::new().await;
    fixture
        .permissions
        .store(Permissions::empty().bits(), Ordering::SeqCst);
    let data = configured_data_at(Some(providers.address));
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceStatus);
    assert!(!invoke_voice_slash_fails(&fixture, command, &data, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    let body = requests
        .iter()
        .find_map(|request| request.body["content"].as_str())
        .expect("member status response");
    assert!(body.contains("configured channel hidden"), "{body}");
    assert!(!body.contains(&CHANNEL.to_string()), "{body}");
    for forbidden in [
        "epoch",
        "model",
        "endpoint",
        "queue",
        "participant",
        "verifier",
    ] {
        assert!(!body.to_ascii_lowercase().contains(forbidden), "{body}");
    }
    assert!(
        !requests
            .iter()
            .any(|request| request.route.contains("health")
                || request.route.contains("models")
                || request.route.contains("chat/completions"))
    );
    assert_eq!(providers.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn actual_unconfigured_voice_status_reaches_explanatory_handler() {
    let fixture = DiscordFixture::new().await;
    let mut data = configured_data();
    data.voice = None;
    let commands = crate::application_commands();
    let command = command_by_key(&commands, CommandKey::VoiceStatus);
    assert!(!invoke_voice_slash_fails(&fixture, command, &data, None).await);
    let requests = fixture.take_requests();
    assert_deferred_first(&requests, command);
    assert!(requests.iter().any(|request| {
        request.body["content"].as_str().is_some_and(|body| {
            body.contains(
                "No voice session is prepared in this server. A manager in a voice channel can use /voice join first.",
            )
        })
    }));
}
