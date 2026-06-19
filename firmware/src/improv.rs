//! Improv Wi-Fi provisioning over BLE (https://www.improv-wifi.com/ble/).
//!
//! In setup mode the device advertises the Improv GATT service as `pixel64`. A browser
//! (Chrome/Edge on desktop or Android — not iOS, which lacks Web Bluetooth) connects, and the
//! user submits their home SSID/password. We auto-authorize (no physical button gate), attempt
//! the Wi-Fi connection while BLE stays up (so we can report status), persist on success, and
//! return the assigned IP. The display shows live status throughout.
//!
//! Wi-Fi is brought up *lazily* — only once credentials arrive — so the BLE discovery/entry
//! phase runs with the radio uncontended (no Wi-Fi/coex starving the GATT stack or the display).

// trouble's `#[gatt_service]` macro expands to code with redundant borrows on each characteristic.
#![allow(clippy::needless_borrows_for_generic_args)]

use core::fmt::Write as _;
use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::Stack;
use embassy_time::Duration;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::WifiController;
use heapless::{String, Vec};
use log::{info, warn};
use trouble_host::prelude::*;

use crate::display::{self, Screen};
use crate::net;
use crate::storage::{CredStore, Credentials};

pub const DEVICE_NAME: &str = "pixel64";

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

/// Where the Wi-Fi station comes from. `Lazy` defers radio bring-up until the first credential
/// attempt (keeping BLE setup radio-clean); `Ready` reuses a stack from a failed boot attempt.
pub enum NetSource {
    Lazy {
        wifi: esp_hal::peripherals::WIFI<'static>,
        spawner: Spawner,
    },
    Ready {
        wifi: WifiController<'static>,
        stack: Stack<'static>,
    },
}

/// Holds the (possibly not-yet-started) Wi-Fi stack and the credential store across attempts.
struct Provisioner<'a> {
    source: Option<NetSource>,
    store: &'a mut CredStore,
}

impl Provisioner<'_> {
    /// Ensure Wi-Fi is up, then connect with the given credentials. Returns the assigned IP.
    async fn connect(&mut self, ssid: &str, password: &str) -> Result<Ipv4Addr, ()> {
        let (mut wifi, stack) = match self.source.take() {
            Some(NetSource::Ready { wifi, stack }) => (wifi, stack),
            Some(NetSource::Lazy { wifi, spawner }) => {
                info!("[improv] starting Wi-Fi for the first connection attempt");
                match net::start(wifi, spawner) {
                    Ok(pair) => pair,
                    Err(e) => {
                        warn!("[improv] Wi-Fi start failed: {:?}", e);
                        return Err(());
                    }
                }
            }
            None => return Err(()),
        };

        let result = net::connect(&mut wifi, &stack, ssid, password).await;
        // Keep the started stack for any retry.
        self.source = Some(NetSource::Ready { wifi, stack });
        result.map_err(|e| warn!("[improv] connect failed: {:?}", e))
    }
}

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

/// Run setup mode until the user provisions Wi-Fi. Returns the live Wi-Fi controller + stack
/// (so the caller keeps them alive) and the assigned IP; credentials are already saved.
pub async fn run_setup(
    controller: ExternalController<BleConnector<'static>, 1>,
    net_source: NetSource,
    store: &mut CredStore,
) -> (WifiController<'static>, Stack<'static>, Ipv4Addr) {
    let mut prov = Provisioner {
        source: Some(net_source),
        store,
    };

    let address = Address::random([0xf0, 0x64, 0x1a, 0x05, 0x8f, 0xff]);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let host = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        mut runner,
        ..
    } = host.build();

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
    let ip = match select(runner.run(), accept_loop(&mut peripheral, &server, &mut prov)).await {
        Either::First(_) => panic!("[improv] BLE runner stopped during setup"),
        Either::Second(ip) => ip,
    };
    // The successful attempt left the started Wi-Fi stack in the provisioner — hand it back so
    // it (and the connection) stays alive after setup completes.
    match prov.source.take() {
        Some(NetSource::Ready { wifi, stack }) => (wifi, stack, ip),
        _ => panic!("[improv] provisioned without a ready Wi-Fi stack"),
    }
}

async fn accept_loop(
    peripheral: &mut Peripheral<'_, ExternalController<BleConnector<'static>, 1>, DefaultPacketPool>,
    server: &Server<'_>,
    prov: &mut Provisioner<'_>,
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
        if let Some(ip) = serve_connection(server, &conn, prov).await {
            return ip;
        }
        info!("[improv] client disconnected without provisioning; re-advertising");
    }
}

async fn serve_connection<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    prov: &mut Provisioner<'_>,
) -> Option<Ipv4Addr> {
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                info!("[improv] client disconnected: {:?}", reason);
                return None;
            }
            GattConnectionEvent::Gatt { event } => {
                // Acknowledge the GATT op; this also commits a (simple or long) write to the store.
                if let Ok(reply) = event.accept() {
                    reply.send().await;
                }
                // The "send Wi-Fi" RPC lands in rpc_command's backing store either way.
                if let Ok(cmd) = server.get(&server.improv.rpc_command)
                    && !cmd.is_empty() {
                        info!("[improv] received {}-byte RPC", cmd.len());
                        let _ = server.set(&server.improv.rpc_command, &Vec::<u8, 128>::new());
                        if let Some(ip) = process_rpc(server, conn, &cmd, prov).await {
                            return Some(ip);
                        }
                    }
            }
            _ => {}
        }
    }
}

async fn process_rpc<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    data: &[u8],
    prov: &mut Provisioner<'_>,
) -> Option<Ipv4Addr> {
    let Some((ssid, password)) = parse_send_wifi(data) else {
        warn!("[improv] malformed send-wifi RPC");
        notify_error(server, conn, ERR_INVALID_RPC).await;
        return None;
    };

    info!("[improv] provisioning Wi-Fi: {}", ssid);
    notify_state(server, conn, STATE_PROVISIONING).await;
    let mut showing: String<32> = String::new();
    let _ = showing.push_str(&ssid);
    display::set_screen(Screen::Connecting(showing));

    match prov.connect(&ssid, &password).await {
        Ok(ip) => {
            info!("[improv] connected, ip = {}", ip);
            let creds = Credentials { ssid, password };
            if let Err(e) = prov.store.save(&creds).await {
                warn!("[improv] failed to persist credentials: {:?}", e);
            }
            let result = build_result(ip);
            let _ = server.set(&server.improv.rpc_result, &result);
            let _ = server.improv.rpc_result.notify(conn, &result).await;
            notify_state(server, conn, STATE_PROVISIONED).await;
            Some(ip)
        }
        Err(()) => {
            notify_error(server, conn, ERR_UNABLE_TO_CONNECT).await;
            notify_state(server, conn, STATE_AUTHORIZED).await; // ready for another attempt
            None
        }
    }
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
fn parse_send_wifi(d: &[u8]) -> Option<(String<32>, String<64>)> {
    if d.len() < 4 || d[0] != CMD_SEND_WIFI {
        return None;
    }
    let datalen = d[1] as usize;
    if d.len() != datalen + 3 {
        return None;
    }
    let checksum = d[d.len() - 1];
    let sum = d[..d.len() - 1].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != checksum {
        return None;
    }
    let ssid_len = d[2] as usize;
    let ssid = core::str::from_utf8(d.get(3..3 + ssid_len)?).ok()?;
    let pw_len = *d.get(3 + ssid_len)? as usize;
    let pw_start = 4 + ssid_len;
    let password = core::str::from_utf8(d.get(pw_start..pw_start + pw_len)?).ok()?;
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
