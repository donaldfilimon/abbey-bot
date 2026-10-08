// Apply as src/commands_help/dispatch_tests/native_follow_up_tests/work_proof.rs.
// Register #[path = "native_follow_up_tests/work_proof.rs"] mod work_proof;
// inside native_follow_up_tests.rs. Reuses its real Poise/HTTP fixture and seed.
use super::*;

#[derive(Clone, Copy, Debug)]
enum WrongProof {
    ActorIdentity,
    ActorBot,
    GuildIdentity,
    ChannelIdentity,
    ChannelGuild,
    CurrentBotHuman,
    ActorUnknownRole,
    BotUnknownRole,
    ExtraAudience,
}

#[tokio::test]
async fn engage_follow_up_native_dispatch_malformed_fresh_work_identity_roles_and_full_audience_refuse_before_source_reads()
 {
    for failure in [
        WrongProof::ActorIdentity,
        WrongProof::ActorBot,
        WrongProof::GuildIdentity,
        WrongProof::ChannelIdentity,
        WrongProof::ChannelGuild,
        WrongProof::CurrentBotHuman,
        WrongProof::ActorUnknownRole,
        WrongProof::BotUnknownRole,
        WrongProof::ExtraAudience,
    ] {
        let fixture = DiscordFixture::new().await;
        let directory = Directory::new();
        let data = native_data(&fixture, &directory);
        let (task, original, _) = seed(&fixture, &data);
        {
            let mut responses = fixture.native_responses.lock().unwrap();
            match failure {
                WrongProof::ActorIdentity => {
                    responses
                        .get_mut(&format!("/guilds/{GUILD}/members/{ACTOR}"))
                        .unwrap()["user"]["id"] = json!(OTHER.to_string())
                }
                WrongProof::ActorBot => {
                    responses
                        .get_mut(&format!("/guilds/{GUILD}/members/{ACTOR}"))
                        .unwrap()["user"]["bot"] = json!(true)
                }
                WrongProof::GuildIdentity => {
                    responses.get_mut(&format!("/guilds/{GUILD}")).unwrap()["id"] =
                        json!((GUILD + 1).to_string())
                }
                WrongProof::ChannelIdentity => {
                    responses.get_mut(&format!("/channels/{CHANNEL}")).unwrap()["id"] =
                        json!((CHANNEL + 1).to_string())
                }
                WrongProof::ChannelGuild => {
                    responses.get_mut(&format!("/channels/{CHANNEL}")).unwrap()["guild_id"] =
                        json!((GUILD + 1).to_string())
                }
                WrongProof::CurrentBotHuman => {
                    responses.get_mut("/users/@me").unwrap()["bot"] = json!(false)
                }
                WrongProof::ActorUnknownRole => {
                    responses
                        .get_mut(&format!("/guilds/{GUILD}/members/{ACTOR}"))
                        .unwrap()["roles"] = json!(["424242"])
                }
                WrongProof::BotUnknownRole => {
                    responses
                        .get_mut(&format!("/guilds/{GUILD}/members/321"))
                        .unwrap()["roles"] = json!(["424242"])
                }
                WrongProof::ExtraAudience => {
                    let mut member = Member::default();
                    member.user = user(OTHER);
                    member.guild_id = GuildId::new(GUILD);
                    responses
                        .get_mut(&format!("/guilds/{GUILD}/members"))
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::to_value(member).unwrap());
                }
            }
        }
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        data.state.request_persistence().await.unwrap();
        let before = std::fs::read(Stores::state_path(&directory.0)).unwrap();
        fixture.take_requests();
        let options = options();
        dispatch(&fixture, &data, &options, &interaction(task))
            .await
            .unwrap();
        let requests = fixture.take_requests();
        let body = assert_private_receipt(&requests, &options);
        assert!(
            body.contains("Task follow-up was not saved.")
                || body == "Discord could not confirm the current permissions. Please try again.",
            "{failure:?}: {body}"
        );
        assert!(
            requests.iter().any(|request| request.method == "GET"),
            "actual fresh Work proof was attempted"
        );
        assert!(
            !requests.iter().any(|request| request
                .route
                .ends_with(&format!("/messages/{SOURCE_MESSAGE}"))
                || request
                    .route
                    .ends_with(&format!("/messages/{RESPONSE_MESSAGE}"))),
            "no private source or response hydration without fresh Work authority: {failure:?}"
        );
        let disk = Stores::load(&directory.0).unwrap();
        assert!(disk.work.engagement.candidates.is_empty());
        assert!(disk.work.engagement.charges.is_empty());
        assert_eq!(disk.work.tasks[&task], original);
        assert_eq!(
            std::fs::read(Stores::state_path(&directory.0)).unwrap(),
            before
        );
        finish(supervisor, writer).await;
    }
}
