# web/

The cloud backend + web UI for pixel64 — a **SvelteKit** app on **Cloudflare** (Workers adapter,
D1 + Drizzle), tooled with **Bun**. The app itself is freshly scaffolded (default `sv` template so
far); the bespoke piece today is the Improv BLE provisioning test client under `improv-test/`.

## Develop

```sh
bun install
bun run dev          # dev server (append -- --open to launch a browser)
bun run build        # production build
bun run preview      # preview the production build
```

See `package.json` for the full script list and `wrangler.jsonc` for the Cloudflare config. To
recreate the scaffold from scratch:

```sh
bun x sv@0.16.1 create --template minimal --types ts \
  --add prettier vitest="usages:unit,component" tailwindcss="plugins:typography,forms" \
  sveltekit-adapter="adapter:cloudflare+cfTarget:workers" drizzle="database:d1" eslint \
  --install bun web
```

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
