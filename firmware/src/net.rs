//! Wi-Fi station bring-up: esp-radio Wi-Fi + embassy-net (DHCP).
//!
//! [`start`] creates the controller + IP stack (not yet connected); [`connect`] applies
//! credentials at runtime, joins, and waits for a DHCP lease. Used by both the boot path
//! (stored creds) and the Improv provisioning path (creds from the phone).

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    sta::StationConfig, Config as WifiConfig, ControllerConfig, Interface, PowerSaveMode,
    WifiController, WifiError,
};
use static_cell::StaticCell;

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

/// Bring up the Wi-Fi station and IP stack. The returned controller is *not* connected yet.
pub fn start(
    wifi: esp_hal::peripherals::WIFI<'static>,
    spawner: Spawner,
) -> Result<(WifiController<'static>, Stack<'static>), WifiError> {
    let iface = Interface::station();
    let mut controller = WifiController::new(wifi, ControllerConfig::default())?;
    // Disable Wi-Fi power save: the radio's periodic sleep/wake (every beacon/DTIM interval)
    // steals CPU from the continuously-refreshed HUB75 panel and shows up as flicker. We're
    // mains-powered, so the extra current draw doesn't matter.
    let _ = controller.set_power_saving(PowerSaveMode::None);

    let rng = Rng::new();
    let seed = ((rng.random() as u64) << 32) | rng.random() as u64;

    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let resources = RESOURCES.init(StackResources::new());

    let (stack, runner) = embassy_net::new(
        iface,
        embassy_net::Config::dhcpv4(Default::default()),
        resources,
        seed,
    );
    spawner.spawn(net_task(runner).unwrap());

    Ok((controller, stack))
}

/// Apply credentials, connect, and wait for a DHCP lease; returns the assigned IPv4 address.
pub async fn connect(
    controller: &mut WifiController<'static>,
    stack: &Stack<'static>,
    ssid: &str,
    password: &str,
) -> Result<core::net::Ipv4Addr, WifiError> {
    controller.set_config(&WifiConfig::Station(
        StationConfig::default()
            .with_ssid(ssid)
            .with_password(password.into()),
    ))?;
    controller.connect_async().await?;
    stack.wait_config_up().await;
    let cfg = stack.config_v4().expect("ipv4 config present after wait_config_up");
    Ok(cfg.address.address())
}
