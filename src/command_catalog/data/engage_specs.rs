//! Registered private engagement controls, expanded beside the core catalog.
macro_rules! engage_catalog {
    ($($rest:tt)*) => { &[
    spec!(EngageFollowUp, Slash, "engage follow_up", BOTH, A0, C0, Engagement, true, "Request a task follow-up."),
    spec!(EngageIntroduce, Slash, "engage introduce", GUILD, A0, C0, Engagement, true, "Propose a mutually approved introduction in this server."),
    spec!(EngageIntroduction, Slash, "engage introduction", GUILD, A0, C0, Engagement, true, "Privately review, edit or withdraw your own introduction."),
    spec!(
        EngageInvite, Slash, "engage invite", BOTH, A0, C0, Engagement, true,
        "Request a policy-gated Activity or voice invitation."
    ),
    spec!(
        EngagePreferences,
        Slash,
        "engage preferences",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Show your private contact settings."
    ),
    spec!(
        EngageConfigure,
        Slash,
        "engage configure",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Save explicit contact limits and timezone."
    ),
    spec!(
        EngageStatus,
        Slash,
        "engage status",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Show your contact policy and delivery blockers."
    ),
    spec!(
        EngageSnooze,
        Slash,
        "engage snooze",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Postpone your existing candidates."
    ),
    spec!(
        EngageStop,
        Slash,
        "engage stop",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Stop personalized contact globally or by scope."
    ),
    spec!(
        EngageResume,
        Slash,
        "engage resume",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Explicitly resume your chosen scope."
    ),
    spec!(
        EngageWeekly,
        Slash,
        "engage weekly",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Choose an explicit scoped weekly subscription."
    ),
    spec!(
        EngageDismiss,
        Slash,
        "engage dismiss",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Dismiss your own pending candidate."
    ),
    spec!(
        EngageFeedback,
        Slash,
        "engage feedback",
        BOTH,
        A0,
        C0,
        Engagement,
        true,
        "Record feedback on your sent delivery."
    ),
    spec!(
        EngageCommunityFeature,
        Slash,
        "engage community feature",
        GUILD,
        A4,
        C0,
        Engagement,
        true,
        "Configure a public feature and explicit channel."
    ),
    spec!(
        EngageCommunityStatus,
        Slash,
        "engage community-status",
        GUILD,
        A4,
        C0,
        Engagement,
        true,
        "Show aggregate public engagement status."
    ),
        $($rest)*
    ] };
}
