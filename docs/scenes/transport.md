# Transport

How the device and the `web/` backend talk: how a running display learns its scene changed, receives
live input values, and fetches bundle/asset bytes. Resolves open question **A1** from
[README.md](README.md).

## Decision

- **DECISION: a persistent WebSocket (over TLS) to a Cloudflare Worker → Durable Object, plus
  on-demand HTTPS for bulk fetch.** Two channels, each to its strength:

  | Channel | Carries | Why |
  |---|---|---|
  | **WebSocket** (persistent, push) | control + live: "scene now = bundle hash X", live input values, small commands/acks | always-connected, tiny messages, one TLS handshake amortized over the device's lifetime |
  | **HTTPS GET** (on-demand, pull) | bulk **bundle/asset bytes by hash** | reuses the mature `reqwless` path; CDN/R2-cacheable; content-addressed so unchanged bytes are never re-sent |

  The device hears "hash X" on the WebSocket, then fetches the bytes over HTTPS **only if it doesn't
  already have them** (content addressing, [bundle-format.md](bundle-format.md)). The persistent
  channel never carries large payloads.

- **DECISION: not long-polling.** On a microcontroller, every poll is a fresh HTTPS request = a fresh
  TLS 1.3 handshake (ECDHE + cert verify, hundreds of ms + RAM churn). A persistent WebSocket pays the
  handshake **once** and then exchanges tiny framed messages — lower latency, less CPU/radio, less
  power. Polling's simplicity is all cloud-side; on the device it's the expensive option.

## Device stack (Embassy)

```
   control messages
   ┌─────────────────────────────────────────────┐
   │ embedded-websocket   RFC 6455: upgrade, client masking, ping/pong
   ├─────────────────────────────────────────────┤
   │ embedded-tls         TLS 1.3, no_std, async        ← the real work
   ├─────────────────────────────────────────────┤
   │ embassy-net TcpSocket   (smoltcp)                  ← mature, easy
   ├─────────────────────────────────────────────┤
   │ cyw43                   (Pico 2 W radio)           ← mature, easy
   └─────────────────────────────────────────────┘
```

Effort, by layer:

- **TCP (`embassy-net`)** — easy; mature async stack, well-exampled on the Pico W.
- **TLS (`embedded-tls`)** — **medium; the long pole.** TLS 1.3-only (Cloudflare speaks 1.3),
  `no_std`, async, integrates with `embassy-net`. Costs ~32 KB RAM for record buffers (records up to
  16 KB each direction) plus handshake scratch. The sharp edge is **certificate validation** (below).
- **WebSocket framing (`embedded-websocket`)** — easy-to-medium; a `no_std` crate handles the upgrade,
  masking, and ping/pong, fed from the TLS stream. Trivial next to TLS.

**Paving caveat:** HTTPS on Embassy is well-paved (`reqwless` is a turnkey HTTPS client over
`embassy-net` + `embedded-tls`). WebSocket-over-TLS is **less paved** — we assemble
`embedded-websocket` + `embedded-tls` ourselves rather than pulling one blessed crate. Doable, just
more novel; the split-channel design keeps that novel path carrying only a few hundred bytes.

- **Placement:** core 0, alongside the (DMA-driven) panel driver and the scheduler/input manager
  ([device-runtime.md](device-runtime.md)). The TLS/WS state machine is comfortable there.
- **RAM:** ~40–50 KB for the whole comms path (TLS buffers dominate). Within budget.

## Cloud side: Worker → Durable Object, with Hibernation

- **One Durable Object per device** (keyed by device id). It holds that device's state, pushes
  "scene = hash X" and live input values down, and receives acks/telemetry up.
- **DECISION: use the WebSocket Hibernation API** (`state.acceptWebSocket()`), **not** an in-memory
  `addEventListener` socket. Hibernation evicts the DO from memory while keeping the socket open,
  waking only on a message. Without it, every connected display pins a DO in memory and bills duration
  24/7 — fatal to an always-connected fleet. With it, an idle device costs almost nothing. **This is
  the feature that makes the always-connected model economical.**
- Bulk bytes are served by a Worker (from **R2**, CDN-cached), addressed by hash — independent of the
  DO.

## Authentication

- **DECISION: server-authenticated TLS + a device bearer token** presented in the WebSocket upgrade
  request (and on HTTPS fetches). Full **mTLS** is heavier and deferred — probably overkill for v1.
- **OPEN (sub-decision 1): certificate validation.** `embedded-tls`'s full chain validation is the
  weak spot. Likely we **pin** Cloudflare's certificate / a known root rather than do general chain
  validation. To be settled when TLS is brought up against the real edge — pinning is simpler and
  safe for a fixed endpoint, at the cost of rotating the pin when CF rotates roots.
- **OPEN (sub-decision 2): device token scheme.** How tokens are minted, delivered (provisioning →
  flash), scoped per-device, and rotated/revoked. Tied to the Wi-Fi onboarding flow.

## Robustness (the real ongoing cost)

Not the protocol — the lifecycle:

- **Reconnect with backoff** on drop; **ping/pong keepalive**; survive Wi-Fi loss and DHCP changes.
- On reconnect, the device reports the bundle hash it's currently showing so the DO can re-sync
  (re-push a newer hash or confirm it's current). Live inputs re-sent or re-fetched on resubscribe.
- Budget engineering time here; it's where embedded networking actually bites, more than the framing.

## Phasing (de-risk TLS first)

1. **HTTPS via `reqwless`** — fetch a bundle by hash. Proves `cyw43` + `embassy-net` + `embedded-tls`
   against the real Cloudflare edge and flushes out cert-validation/buffer issues on the paved path.
2. **WebSocket → Durable Object** — add the control/live channel on top of the now-proven TLS stack.

This validates the scary part (TLS) before also taking on the less-paved WS assembly.

## Open questions

1. **Certificate validation** — pin vs. full chain validation (sub-decision 1 above).
2. **Device token scheme** — minting/delivery/rotation, tied to onboarding (sub-decision 2 above).
3. **Message schema** for the WebSocket control channel (the concrete shapes of "scene changed", live
   input batches, acks, telemetry) — to be specified alongside the first backend implementation.
