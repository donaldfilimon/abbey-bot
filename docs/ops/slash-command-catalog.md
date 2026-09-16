# Abbey slash command catalog

Canonical registration policy lives in `src/command_catalog/` (`REGISTERED`).
`commands_help::bind_commands` copies leaf names, contexts, ephemeral flags, and
`default_member_permissions` onto poise adapters before global/guild publish.

## Permission model

| Layer | Role |
|---|---|
| Discord `default_member_permissions` | UX hide/show in the client |
| Catalog access rules (`AccessId`) | Runtime guard after early defer (3s) |
| **Permission-mirror** (`permission_mirror`) | For `/server` mutations: requester **and** bot must hold the action bit; fail closed |

`/admin act on|off` only toggles **unsolicited replies** (learning pipeline). It does **not** authorize guild mutations.

### `/server` actions (permission-mirror)

| Command | Required bit (both sides) | Notes |
|---|---|---|
| `server blueprint` | — | Emit-only plan; no mutate |
| `server create-channel` | Manage Channels | Non-destructive |
| `server rename-channel` | Manage Channels | Non-destructive |
| `server slowmode` | Manage Channels | 0–21600s |
| `server delete-channel` | Manage Channels | Needs `confirm:true` |
| `server assign-role` / `remove-role` | Manage Roles | Never deletes roles |
| `server move-member` | Move Members | Voice/stage destination |
| `server purge` | Manage Messages | Needs `confirm:true`; 2–100 msgs |

Hard rules: never delete roles with holders > 0; never flatten a live guild to a Community blueprint from slash commands (CLI `--server-plan` remains additive).

## Early acknowledgement

`catalog_check` defers (ephemeral when `private`) before permission/capability
REST so Discord's 3s interaction window cannot expire. Slow work (voice music
Action Rows, server mutations) must keep that pattern: ack first, then REST.

## Groups

- `/persona`, `/pending`, `/forum`, `/admin`, `/voice`, `/server` — parents are wiring only; leaves are catalogued.
- Context menus: `Abbey: profile`, `Ask Abbey`, memory menu.
