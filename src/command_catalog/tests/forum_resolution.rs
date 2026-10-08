use super::*;

#[test]
fn forum_resolution_is_private_member_discoverable_guild_only() {
    let command = command(CommandKey::ForumResolve);
    assert_eq!(command.name, "forum resolve");
    assert_eq!(command.eligibility.access, AccessId::A0);
    assert!(command.private);
    assert_eq!(availability(command, &member()), Availability::Ready);
    assert!(render_help(HelpSection::Server, &member()).contains("`/forum resolve`"));
    let mut dm = member();
    dm.context = InteractionContext::BotDm;
    assert!(matches!(
        availability(command, &dm),
        Availability::AccessBlocked(_)
    ));
    assert!(!render_help(HelpSection::Server, &dm).contains("`/forum resolve`"));
}

#[test]
fn forum_resolution_registration_has_choices_and_no_blanket_manager_permission() {
    let parent = crate::commands_forum::forum();
    let resolve = parent
        .subcommands
        .iter()
        .find(|command| command.name == "resolve")
        .unwrap();
    assert!(resolve.guild_only);
    assert!(resolve.default_member_permissions.is_empty());
    assert!(resolve.required_permissions.is_empty());
    let choices: Vec<_> = resolve
        .parameters
        .iter()
        .find(|parameter| parameter.name == "status")
        .unwrap()
        .choices
        .iter()
        .map(|choice| choice.name.as_str())
        .collect();
    assert_eq!(choices, ["Solved", "Unresolved"]);
    assert!(
        resolve
            .parameters
            .iter()
            .any(|parameter| parameter.name == "thread" && !parameter.required)
    );
}
