# web/

The cloud backend + web UI for pixel64. The full app (Svelte 5 / SvelteKit on Cloudflare, tooled
with Bun) is **not scaffolded yet** — see the repo root `CLAUDE.md`.

What's here today:

## `improv-test/` — Improv-over-BLE provisioning test client

A zero-build, self-contained Web Bluetooth page for exercising the firmware's Improv provisioning
without depending on `improv-wifi.com` (whose hosted SDK version/cache we don't control). It also
**A/B-tests the macOS write bug**: a checkbox switches the credential write between
`writeValueWithResponse()` (works everywhere) and `writeValueWithoutResponse()` (silently dropped by
Chrome on macOS — [improv-wifi/sdk-ble-js#213](https://github.com/improv-wifi/sdk-ble-js/issues/213),
fixed upstream in PR #217). See `firmware/docs/pico-port.md` for the full diagnosis.

### Run it

Web Bluetooth only works in a **secure context** — `http://localhost` or HTTPS — and only in
**Chrome/Edge** (desktop or Android; not Safari/iOS). Serve the folder over localhost:

```sh
# Bun (the project's tooling):
bunx serve web/improv-test
# …or anything else that serves static files on localhost:
cd web/improv-test && python3 -m http.server 8000
```

Then open the printed `http://localhost:<port>` in Chrome, **Connect to pixel64…**, enter Wi-Fi
credentials, and **Send**. Watch `current_state` advance Provisioning → Provisioned and a device URL
appear. The on-page log shows every notification and the exact bytes written.

### Using it to confirm the macOS diagnosis

1. Leave the checkbox **unchecked** (write-with-response) → provisioning should complete on macOS.
2. **Check** "Use `writeValueWithoutResponse()`" and resend → on macOS Chrome the write is dropped
   (the device's serial shows no `received … RPC`), reproducing the original hang. On Android/Linux
   Chrome it still works — which is exactly why the bug is macOS-only.

This pins the failure to the browser's write path, not the firmware.
