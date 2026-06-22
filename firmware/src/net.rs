//! Wi-Fi station connect: join + DHCP over the cyw43 `Control` and the embassy-net `Stack`.
//!
//! Used by both the boot path (stored credentials) and the Improv provisioning path (creds from the
//! browser). On the cyw43 the radio and IP stack are already up — this just applies credentials and
//! waits for a lease.

use core::net::Ipv4Addr;

use cyw43::{Control, JoinOptions};
use embassy_futures::select::{select, Either};
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use log::warn;

/// How long to wait for association before giving up. cyw43's `join` doesn't reliably time out on a
/// bad password (it can block ~indefinitely), so we bound it ourselves.
const JOIN_TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for a DHCP lease after associating before giving up.
const DHCP_TIMEOUT: Duration = Duration::from_secs(15);

/// Join `ssid` and wait for a DHCP lease; returns the assigned IPv4 address. Bounded by timeouts so
/// bad credentials surface as `Err(())` rather than hanging.
pub async fn connect(
    control: &mut Control<'static>,
    stack: Stack<'static>,
    ssid: &str,
    password: &str,
) -> Result<Ipv4Addr, ()> {
    // Start from a clean state. cyw43's `join` has no internal timeout, so a previous attempt that
    // we abandoned on `JOIN_TIMEOUT` (or that the firmware left mid-association) can make the next
    // join hang — `leave()` (disassociate) resets it. Harmless if not currently associated.
    control.leave().await;

    match select(
        control.join(ssid, JoinOptions::new(password.as_bytes())),
        Timer::after(JOIN_TIMEOUT),
    )
    .await
    {
        Either::First(Ok(())) => {}
        Either::First(Err(e)) => {
            warn!("[net] join failed: {:?}", e);
            return Err(());
        }
        Either::Second(()) => {
            warn!("[net] join timed out after {}s", JOIN_TIMEOUT.as_secs());
            return Err(());
        }
    }
    match select(stack.wait_config_up(), Timer::after(DHCP_TIMEOUT)).await {
        Either::First(()) => {}
        Either::Second(()) => {
            warn!("[net] DHCP timed out after associating");
            return Err(());
        }
    }
    stack.config_v4().map(|c| c.address.address()).ok_or(())
}
