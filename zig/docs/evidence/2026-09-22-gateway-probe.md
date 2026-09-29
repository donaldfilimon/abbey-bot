# Token-free gateway probe, 2026-09-22

Command: `./zig-out/bin/abbey-bot-zig gateway-probe` (commit that adds
`src/gateway/probe.zig`), run on Donald's Mac from the repository root.

What it does: `GET https://discord.com/api/v10/gateway` through
`std.http.Client` (system CA bundle via `Certificate.Bundle.rescan`), then a
TLS connection to the returned `wss://` host, the RFC 6455 opening handshake
from `src/gateway/ws.zig`, one text frame read (Hello, op 10), and a 1000
close. It never sends Identify, carries no token, and cannot affect any bot.

Observed output:

```
gateway-probe: ok (gateway url wss: true, hello heartbeat_interval 41250 ms)
PROBE-EXIT:0
```

This is a dated observation of live TLS and WebSocket interoperability with
Discord's production edge. It is not a test and does not establish Identify,
Resume, command registration, or any behavior that needs a bot token.
