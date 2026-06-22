//! Improv Wi-Fi provisioning over BLE (https://www.improv-wifi.com/ble/), on the cyw43 radio.
//!
//! In setup mode the device advertises the Improv GATT service as `pixel64`. A browser
//! (Chrome/Edge on desktop or Android — not iOS, which lacks Web Bluetooth) connects and the user
//! submits their home SSID/password. We auto-authorize (no physical button gate), then join Wi-Fi
//! **while the BLE link stays up** (so we can report status), and return the assigned IP.
//!
//! This is the trouble-host 0.6 GATT code ported from the ESP build essentially verbatim — only
//! the BLE controller (esp-radio `BleConnector` → cyw43 `BtDriver`) and the Wi-Fi path changed.
//! On the cyw43 there's a single radio: `control` joins Wi-Fi, `bt_device` is the BLE transport,
//! and the embassy-net `Stack` is created up front — so the ESP's lazy-radio `NetSource` dance is
//! gone, and joining Wi-Fi while BLE is connected is the very concurrency we're spiking here.
//!
//! NOTE (M2c spike): credential persistence is stubbed — storage is milestone M4. See
//! docs/pico-port.md.

// trouble's #[gatt_service] macro expands to code with redundant borrows on each characteristic.
#![allow(clippy::needless_borrows_for_generic_args)]

use core::fmt::Write as _;
use core::net::Ipv4Addr;

use cyw43::bluetooth::BtDriver;
use cyw43::Control;
use embassy_futures::select::{select, Either};
use embassy_net::Stack;
use embassy_time::Duration;
use heapless::{String, Vec};
use log::{info, warn};
use trouble_host::prelude::*;

use crate::display::{self, Screen};
use crate::net;
use crate::storage::{CredStore, Credentials};

pub const DEVICE_NAME: &str = "pixel64";

/// The BLE controller type: cyw43's BT transport behind bt-hci's ExternalController.
type BleController = ExternalController<BtDriver<'static>, 1>;

// Improv current-state values.
const STATE_AUTHORIZED: u8 = 0x02;
const STATE_PROVISIONING: u8 = 0x03;
const STATE_PROVISIONED: u8 = 0x04;
// Improv error-state values.
const ERR_NONE: u8 = 0x00;
const ERR_INVALID_RPC: u8 = 0x01;
const ERR_UNABLE_TO_CONNECT: u8 = 0x03;
// Improv RPC command ids.
const CMD_SEND_WIFI: u8 = 0x01;

/// Improv service UUID `00467768-6228-2272-4663-277478268000`, little-endian for advertising.
const IMPROV_UUID_LE: [u8; 16] = [
    0x00, 0x80, 0x26, 0x78, 0x74, 0x27, 0x63, 0x46, 0x72, 0x22, 0x28, 0x62, 0x68, 0x77, 0x46, 0x00,
];

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

#[gatt_server]
struct Server {
    improv: ImprovService,
}

#[gatt_service(uuid = "00467768-6228-2272-4663-277478268000")]
struct ImprovService {
    #[characteristic(uuid = "00467768-6228-2272-4663-277478268001", read, notify)]
    current_state: u8,
    #[characteristic(uuid = "00467768-6228-2272-4663-277478268002", read, notify)]
    error_state: u8,
    // `read` so server.get() can pull the value the client wrote (incl. via a long write).
    #[characteristic(uuid = "00467768-6228-2272-4663-277478268003", read, write)]
    rpc_command: Vec<u8, 128>,
    #[characteristic(uuid = "00467768-6228-2272-4663-277478268004", read, notify)]
    rpc_result: Vec<u8, 96>,
    #[characteristic(uuid = "00467768-6228-2272-4663-277478268005", read)]
    capabilities: u8,
}

/// Run setup mode until the user provisions Wi-Fi; returns the assigned IP. The BLE link stays up
/// across the Wi-Fi join so status notifications reach the Improv client.
pub async fn run_setup(
    bt_device: BtDriver<'static>,
    control: &mut Control<'static>,
    stack: Stack<'static>,
    store: &mut CredStore,
) -> Ipv4Addr {
    let controller: BleController = ExternalController::new(bt_device);
    let address = Address::random([0xf0, 0x64, 0x1a, 0x05, 0x8f, 0xff]);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    // `build()` borrows the stack, so `ble_stack` stays alive — we need a `&` to it to answer
    // connection-parameter requests (the macOS link-stall fix). Named `ble_stack` to avoid clashing
    // with the embassy-net Wi-Fi `Stack`.
    let ble_stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        mut runner,
        ..
    } = ble_stack.build();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();

    // Auto-authorized, no identify button, no error.
    let _ = server.set(&server.improv.current_state, &STATE_AUTHORIZED);
    let _ = server.set(&server.improv.capabilities, &0u8);
    let _ = server.set(&server.improv.error_state, &ERR_NONE);

    info!("[improv] setup mode: advertising as {}", DEVICE_NAME);
    match select(
        runner.run(),
        accept_loop(&mut peripheral, &server, control, stack, &ble_stack, store),
    )
    .await
    {
        Either::First(_) => panic!("[improv] BLE runner stopped during setup"),
        Either::Second(ip) => ip,
    }
}

async fn accept_loop(
    peripheral: &mut Peripheral<'_, BleController, DefaultPacketPool>,
    server: &Server<'_>,
    control: &mut Control<'static>,
    stack: Stack<'static>,
    ble_stack: &trouble_host::Stack<'_, BleController, DefaultPacketPool>,
    store: &mut CredStore,
) -> Ipv4Addr {
    let mut adv_data = [0u8; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids128(&[IMPROV_UUID_LE]),
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut adv_data[..],
    )
    .unwrap();

    let params = AdvertisementParameters {
        interval_min: Duration::from_millis(100),
        interval_max: Duration::from_millis(100),
        ..Default::default()
    };

    loop {
        let advertiser = peripheral
            .advertise(
                &params,
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..adv_len],
                    scan_data: &[],
                },
            )
            .await
            .unwrap();
        let conn = match advertiser.accept().await {
            Ok(c) => match c.with_attribute_server(server) {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("[improv] attribute server error: {:?}", e);
                    continue;
                }
            },
            Err(e) => {
                warn!("[improv] accept error: {:?}", e);
                continue;
            }
        };
        info!("[improv] client connected");
        if let Some(ip) = serve_connection(server, &conn, control, stack, ble_stack, store).await {
            return ip;
        }
        info!("[improv] client disconnected without provisioning; re-advertising");
        display::set_screen(Screen::Setup); // clear any "joining"/"failed" screen from this attempt
    }
}

async fn serve_connection<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    control: &mut Control<'static>,
    stack: Stack<'static>,
    ble_stack: &trouble_host::Stack<'_, BleController, P>,
    store: &mut CredStore,
) -> Option<Ipv4Addr> {
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                info!("[improv] client disconnected: {:?}", reason);
                return None;
            }
            // macOS CoreBluetooth sends a connection-parameter update request right after
            // connecting; it MUST be answered or the link stalls and the credential write never
            // arrives (the macOS provisioning bug). Accept the peer's requested params.
            GattConnectionEvent::RequestConnectionParams(req) => {
                info!("[improv] connection-params request — accepting");
                if let Err(e) = req.accept(None, ble_stack).await {
                    warn!("[improv] failed to accept connection params: {:?}", e);
                }
            }
            GattConnectionEvent::ConnectionParamsUpdated { conn_interval, .. } => {
                info!("[improv] connection params updated (interval {:?})", conn_interval);
            }
            GattConnectionEvent::PhyUpdated { .. } => info!("[improv] phy updated"),
            GattConnectionEvent::DataLengthUpdated { .. } => info!("[improv] data length updated"),
            GattConnectionEvent::Gatt { event } => {
                // Acknowledge the GATT op; this also commits a (simple or long) write to the store.
                if let Ok(reply) = event.accept() {
                    reply.send().await;
                }
                // The "send Wi-Fi" RPC lands in rpc_command's backing store either way.
                if let Ok(cmd) = server.get(&server.improv.rpc_command)
                    && !cmd.is_empty()
                {
                    info!("[improv] received {}-byte RPC", cmd.len());
                    let _ = server.set(&server.improv.rpc_command, &Vec::<u8, 128>::new());
                    if let Some(ip) = process_rpc(server, conn, &cmd, control, stack, store).await {
                        return Some(ip);
                    }
                }
            }
            // The remaining variants only exist with the `security` feature, which we don't enable.
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
}

async fn process_rpc<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    data: &[u8],
    control: &mut Control<'static>,
    stack: Stack<'static>,
    store: &mut CredStore,
) -> Option<Ipv4Addr> {
    let Some((ssid, password)) = parse_send_wifi(data) else {
        warn!("[improv] malformed send-wifi RPC");
        notify_error(server, conn, ERR_INVALID_RPC).await;
        return None;
    };

    // A WPA2 passphrase is 8–63 chars (or empty for an open network). cyw43's `join` HANGS on an
    // invalid-length passphrase rather than returning an error (its auth event never arrives), so
    // reject it up front instead of feeding it to the driver.
    if !password.is_empty() && !(8..=63).contains(&password.len()) {
        warn!("[improv] invalid password length ({}); rejecting", password.len());
        fail(server, conn, "bad password").await;
        return None;
    }

    info!("[improv] provisioning Wi-Fi: {}", ssid);
    display::set_screen(Screen::Connecting(ssid.clone()));
    notify_state(server, conn, STATE_PROVISIONING).await;

    // We join Wi-Fi here while the BLE GATT link is still connected, and the notify_*() calls below
    // must reach the client over that same link.
    match net::connect(control, stack, &ssid, &password).await {
        Ok(ip) => {
            info!("[improv] connected, ip = {}", ip);
            // Persist so this network is rejoined on the next boot without re-provisioning.
            let creds = Credentials { ssid, password };
            match store.save(&creds).await {
                Ok(()) => info!("[improv] credentials persisted"),
                Err(e) => warn!("[improv] failed to persist credentials: {:?}", e),
            }
            let result = build_result(ip);
            let _ = server.set(&server.improv.rpc_result, &result);
            let _ = server.improv.rpc_result.notify(conn, &result).await;
            notify_state(server, conn, STATE_PROVISIONED).await;
            Some(ip)
        }
        Err(()) => {
            warn!("[improv] Wi-Fi connect failed");
            fail(server, conn, "try again").await;
            None
        }
    }
}

/// Report a provisioning failure: Improv error + back to authorized, and a `FAILED` screen (clearing
/// any stuck "joining" screen) so the panel never looks hung.
async fn fail<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>, msg: &str) {
    notify_error(server, conn, ERR_UNABLE_TO_CONNECT).await;
    notify_state(server, conn, STATE_AUTHORIZED).await; // ready for another attempt
    let mut s: String<24> = String::new();
    let _ = s.push_str(msg);
    display::set_screen(Screen::Failed(s));
}

async fn notify_state<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>, s: u8) {
    let c = server.improv.current_state;
    let _ = server.set(&c, &s);
    let _ = c.notify(conn, &s).await;
}

async fn notify_error<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>, e: u8) {
    let c = server.improv.error_state;
    let _ = server.set(&c, &e);
    let _ = c.notify(conn, &e).await;
}

/// Parse Improv "send Wi-Fi credentials": `[0x01][datalen][ssidlen][ssid][pwlen][pw][checksum]`.
///
/// The `datalen` byte (index 1) is **reconstructed from the self-delimiting structure** rather than
/// trusted, then the whole packet is checksum-validated. This tolerates a cyw43 BLE receive-path
/// corruption that reproducibly decrements byte[1] by one (see docs/pico-port.md §"cyw43 BLE
/// byte-1 corruption"), while the Improv checksum still guarantees the SSID/password are intact —
/// any real cred corruption fails the checksum and is rejected, so the client just retries. We
/// never accept creds the checksum doesn't cover.
fn parse_send_wifi(d: &[u8]) -> Option<(String<32>, String<64>)> {
    if d.len() < 5 || d[0] != CMD_SEND_WIFI {
        warn!(
            "[improv] parse: bad header (len={}, cmd={:#04x})",
            d.len(),
            d.first().copied().unwrap_or(0)
        );
        return None;
    }
    // Parse the self-delimiting structure (ssidlen/pwlen), ignoring the possibly-corrupt datalen.
    let ssid_len = d[2] as usize;
    let ssid = core::str::from_utf8(d.get(3..3 + ssid_len)?).ok()?;
    let pw_len = *d.get(3 + ssid_len)? as usize;
    let pw_start = 4 + ssid_len;
    let password = core::str::from_utf8(d.get(pw_start..pw_start + pw_len)?).ok()?;

    // The packet must end exactly at the checksum: cmd+datalen+ssidlen+ssid+pwlen+pw+checksum.
    let expected_total = 5 + ssid_len + pw_len;
    if d.len() != expected_total {
        warn!(
            "[improv] parse: structure doesn't fit (len {}, structure implies {})",
            d.len(),
            expected_total
        );
        return None;
    }

    // Reconstruct datalen (= ssidlen byte + ssid + pwlen byte + pw) and validate the Improv
    // checksum using it. The sender computed the checksum with the correct datalen, so a match
    // proves the SSID/password are intact regardless of a corrupted byte[1].
    let datalen = (ssid_len + pw_len + 2) as u8;
    let mut sum = d[0].wrapping_add(datalen);
    for &b in &d[2..d.len() - 1] {
        sum = sum.wrapping_add(b);
    }
    let checksum = d[d.len() - 1];
    if sum != checksum {
        warn!(
            "[improv] parse: checksum mismatch (calc {:#04x}, got {:#04x}) — rejecting, client should retry",
            sum, checksum
        );
        return None;
    }
    if d[1] != datalen {
        warn!(
            "[improv] datalen byte arrived {:#04x}, reconstructed {:#04x} (known cyw43 BLE byte-1 \
             corruption); creds verified intact by checksum",
            d[1], datalen
        );
    }
    Some((String::try_from(ssid).ok()?, String::try_from(password).ok()?))
}

/// Build the Improv result payload carrying the device URL: `[0x01][datalen][urllen][url][checksum]`.
fn build_result(ip: Ipv4Addr) -> Vec<u8, 96> {
    let o = ip.octets();
    let mut url: String<32> = String::new();
    let _ = write!(url, "http://{}.{}.{}.{}", o[0], o[1], o[2], o[3]);
    let url = url.as_bytes();

    let mut v: Vec<u8, 96> = Vec::new();
    let _ = v.push(CMD_SEND_WIFI);
    let _ = v.push((1 + url.len()) as u8);
    let _ = v.push(url.len() as u8);
    let _ = v.extend_from_slice(url);
    let checksum = v.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    let _ = v.push(checksum);
    v
}
