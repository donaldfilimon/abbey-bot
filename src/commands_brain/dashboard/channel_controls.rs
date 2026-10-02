//! Current-channel participation effects; memory consent is a separate contract.
use super::*;

pub(super) fn buttons(session: &crate::admin_dashboard::AdminSession) -> Vec<CreateButton> {
    use crate::admin_dashboard::AdminAction as A;
    [true, false]
        .into_iter()
        .map(|allow| {
            CreateButton::new(session.custom_id(A::SetChannelParticipation(allow)))
                .label(if allow {
                    "Allow this channel"
                } else {
                    "Block this channel"
                })
                .style(ButtonStyle::Secondary)
        })
        .collect()
}

pub(super) fn change(settings: &mut GuildSettings, channel: &str, allow: bool) -> bool {
    if settings.unsolicited_channels.is_none() {
        settings.unsolicited_channels = Some(Default::default());
        if allow {
            settings
                .unsolicited_channels
                .as_mut()
                .unwrap()
                .insert(channel.into());
        }
        return true;
    }
    let channels = settings.unsolicited_channels.as_mut().unwrap();
    if allow {
        channels.insert(channel.into())
    } else {
        channels.remove(channel)
    }
}

pub(super) fn status(settings: &GuildSettings, channel: &str) -> String {
    match &settings.unsolicited_channels {
        None => format!(
            "Channel scope: {channel}. Legacy eligibility includes all channels; the first allow/block sets an explicit channel list."
        ),
        Some(channels) => format!(
            "Channel scope: {channel}. Unsolicited eligibility: {}. Explicit list contains {} channels. Guild opt-in, quiet, provider readiness and rate limits still apply; this setting grants no memory or voice consent.",
            if channels.contains(channel) {
                "allowed"
            } else {
                "blocked"
            },
            channels.len()
        ),
    }
}

pub(super) async fn apply(
    data: &crate::Data,
    guild_id: u64,
    channel_id: u64,
    allow: bool,
) -> String {
    let channel = guild::scoped_channel_id(PLATFORM, &channel_id.to_string());
    let mut changed = false;
    let mut legacy = false;
    update_dashboard_setting(data, guild_id, |settings| {
        legacy = settings.unsolicited_channels.is_none();
        changed = change(settings, &channel, allow);
    });
    if !changed {
        return format!(
            "{channel} already has the requested participation eligibility. No new persistence requested."
        );
    }
    let admission = if allow { "allowed" } else { "blocked" };
    let replacement = if legacy {
        " Legacy all-channel eligibility was replaced by an explicit list; other channels are now blocked."
    } else {
        ""
    };
    let persistence = match data.state.request_persistence().await {
        Ok(report) => render_persistence_result(&report),
        Err(error) => error.to_string(),
    };
    format!(
        "Applied to running bot: guild {guild_id}, channel {channel}, participation {admission}.{replacement}\n{persistence}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_action_replaces_legacy_without_widening_other_channels() {
        for allow in [true, false] {
            let mut settings = GuildSettings::default();
            assert!(change(&mut settings, "discord:c", allow));
            assert_eq!(settings.unsolicited_channel_allowed("discord:c"), allow);
            assert!(!settings.unsolicited_channel_allowed("discord:other"));
            assert!(!change(&mut settings, "discord:c", allow));
            assert!(settings.unsolicited_channels.is_some());
        }
    }
}
