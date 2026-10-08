//! Adjacent work command specifications, preserving engagement-before-work catalog order.
macro_rules! work_catalog {
    ($($rest:tt)*) => { engage_catalog![
    spec!(
        WorkProject,
        Slash,
        "work project",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Create a personal or channel project."
    ),
    spec!(
        WorkProjects,
        Slash,
        "work projects",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "List projects available here."
    ),
    spec!(
        WorkGoal,
        Slash,
        "work goal",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Record a project goal."
    ),
    spec!(
        WorkTask,
        Slash,
        "work task",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Create an assigned task."
    ),
    spec!(
        WorkDecision,
        Slash,
        "work decision",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Record a project decision."
    ),
    spec!(
        WorkRecallConfigure,
        Slash,
        "work recall configure",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Configure scope recall opt-in."
    ),
    spec!(
        WorkRecallShow,
        Slash,
        "work recall show",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Inspect scope recall availability and revision."
    ),
    spec!(WorkContinuityShow, Slash, "work continuity show", BOTH, A0, C0, Work, true, "Show a card."),
    spec!(WorkContinuityPropose, Slash, "work continuity propose", BOTH, A0, C0, Work, true, "Preview a card."),
    spec!(WorkContinuityConfirm, Slash, "work continuity confirm", BOTH, A0, C0, Work, true, "Confirm a card."),
    spec!(WorkContinuityClear, Slash, "work continuity clear", BOTH, A0, C0, Work, true, "Clear a card."),
    spec!(
        WorkBriefing,
        Slash,
        "work briefing",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Review current work privately."
    ),
    spec!(
        WorkComplete,
        Slash,
        "work complete",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Complete a current task."
    ),
    spec!(
        WorkStatus,
        Slash,
        "work status",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Change a task's status."
    ),
    spec!(
        WorkSnooze,
        Slash,
        "work snooze",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Snooze a task until a chosen time."
    ),
    spec!(
        WorkMember,
        Slash,
        "work member",
        GUILD,
        A0,
        C0,
        Work,
        true,
        "Manage a shared project's members."
    ),
    spec!(
        WorkPreferences,
        Slash,
        "work preferences",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Inspect your learning settings and evidence."
    ),
    spec!(
        WorkResetPreferences,
        Slash,
        "work reset_preferences",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Clear your learned preferences and evidence."
    ),
    spec!(
        WorkLearning,
        Slash,
        "work learning",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Enable or disable scope preference learning."
    ),
    spec!(
        WorkTiming,
        Slash,
        "work timing",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Set an explicit optional delivery hour."
    ),
    spec!(
        WorkFeedback,
        Slash,
        "work feedback",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Record or correct your attributable delivery feedback."
    ),
    spec!(
        WorkAutomation,
        Slash,
        "work automation",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Configure scope automation opt-in and delivery limits."
    ),
    spec!(
        WorkReminder,
        Slash,
        "work reminder",
        BOTH,
        A0,
        C0,
        Work,
        true,
        "Set or cancel an explicit task reminder."
    ),
        $($rest)*
    ] };
}
