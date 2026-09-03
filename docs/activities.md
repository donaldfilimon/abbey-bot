# Discord Activities (in-voice apps)

Abbey is an Activities-enabled app. The auto-created Entry Point command
`launch` (type Primary Entry Point, handler Discord Launch Activity) is the
primary way members start Abbey from the App Launcher / rocket in a voice
channel. Global command registration must keep that Entry Point
(`register_globally_keeping_entry_point`); deleting it disables the Activity.

## What works

- Launch Abbey as an Activity while in a voice channel (Office Hours and other
  VCs with Use Embedded Activities).
- Channel overwrites need View Channel, Connect, Speak, **Stream**, and
  **Use Embedded Activities** for Member and Abbey. Live join fails closed if
  Abbey is missing those bits.
- Conversational capture is still consent-gated: `/voice join consent:true`
  from an in-channel manager. AUTOJOIN is muted presence only.

## What Discord does not allow

Bots cannot classic user **Go Live** / desktop screenshare. Do not expect Abbey
to appear as a Go Live stream. Shared visual experiences go through Activities
(Embedded App SDK), not bot screenshare.

## Operator checks

1. In Office Hours, confirm Member can Use Activities.
2. Rocket / App Launcher shows Abbey; Entry Point `launch` still exists globally.
3. `/voice status` then `/voice join consent:true` for spoken turns.
