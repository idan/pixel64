//! Persistent Wi-Fi credential storage (RP2350 flash).
//!
//! Stores the SSID + password using [`sequential_storage`] (CRC-checked, power-fail safe) in a
//! small region reserved at the **top** of the Pico 2 W's 4 MiB flash. There's no partition table
//! here (that was an esp-idf thing) — we just pick a fixed flash-relative range that memory.x keeps
//! out of the linker's FLASH window. embassy-rp's flash driver is blocking, so we adapt it to
//! sequential-storage's async API with `BlockingAsync`.
//!
//! KEEP IN SYNC: memory.x reserves the matching 16 KiB at the top of flash.

use core::ops::Range;

use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_rp::flash::{Blocking, Flash as RpFlash};
use embassy_rp::peripherals::FLASH;
use embassy_rp::Peri;
use heapless::String;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapConfig, MapStorage};

/// Total flash on the Pico 2 W.
const FLASH_SIZE: usize = 4 * 1024 * 1024;
/// Credentials region: the top 16 KiB (4 erase sectors). KEEP IN SYNC with memory.x.
const CREDS_LEN: u32 = 16 * 1024;
const CREDS_RANGE: Range<u32> = (FLASH_SIZE as u32 - CREDS_LEN)..FLASH_SIZE as u32;

/// Map key under which the single serialized credentials blob is stored.
const CREDS_KEY: u8 = 0;

/// Wi-Fi limits: SSID up to 32 bytes, WPA2 passphrase up to 63 (+1 for slack).
pub const MAX_SSID: usize = 32;
pub const MAX_PASS: usize = 64;

/// A scratch buffer big enough for the serialized blob plus sequential-storage's item framing.
const SCRATCH: usize = 1 + MAX_SSID + 1 + MAX_PASS + 32;

/// Decoded Wi-Fi credentials.
#[derive(Clone)]
pub struct Credentials {
    pub ssid: String<MAX_SSID>,
    pub password: String<MAX_PASS>,
}

impl Credentials {
    /// Pack as `[ssid_len][ssid][pass_len][pass]`; returns the used length.
    fn serialize(&self, out: &mut [u8]) -> usize {
        let s = self.ssid.as_bytes();
        let p = self.password.as_bytes();
        out[0] = s.len() as u8;
        out[1..1 + s.len()].copy_from_slice(s);
        out[1 + s.len()] = p.len() as u8;
        let pstart = 2 + s.len();
        out[pstart..pstart + p.len()].copy_from_slice(p);
        pstart + p.len()
    }

    /// Inverse of [`serialize`]; returns `None` on any malformed/oversized field.
    fn parse(b: &[u8]) -> Option<Self> {
        let slen = *b.first()? as usize;
        let ssid = core::str::from_utf8(b.get(1..1 + slen)?).ok()?;
        let plen = *b.get(1 + slen)? as usize;
        let pstart = 2 + slen;
        let password = core::str::from_utf8(b.get(pstart..pstart + plen)?).ok()?;
        Some(Self {
            ssid: String::try_from(ssid).ok()?,
            password: String::try_from(password).ok()?,
        })
    }
}

#[derive(Debug)]
pub enum StorageError {
    /// A flash read/write/erase failed.
    Flash,
}

type Flash = BlockingAsync<RpFlash<'static, FLASH, Blocking, FLASH_SIZE>>;

/// Owns the flash and serves the credential store at [`CREDS_RANGE`].
pub struct CredStore {
    flash: Flash,
}

impl CredStore {
    /// Take the flash peripheral. (Infallible — unlike the esp build, there's no partition table
    /// to parse; the region is a fixed constant.)
    pub fn new(flash: Peri<'static, FLASH>) -> Self {
        Self {
            flash: BlockingAsync::new(RpFlash::new_blocking(flash)),
        }
    }

    /// Load saved credentials, or `None` if unprovisioned (or on a recoverable read error).
    pub async fn load(&mut self) -> Option<Credentials> {
        let mut buf = [0u8; SCRATCH];
        let mut map =
            MapStorage::<u8, _, _>::new(&mut self.flash, MapConfig::new(CREDS_RANGE), NoCache::new());
        match map.fetch_item::<&[u8]>(&mut buf, &CREDS_KEY).await {
            Ok(Some(blob)) => Credentials::parse(blob),
            Ok(None) => None,
            Err(e) => {
                log::warn!("[storage] load error: {:?}", e);
                None
            }
        }
    }

    /// Persist credentials (overwrites any previous value).
    pub async fn save(&mut self, creds: &Credentials) -> Result<(), StorageError> {
        let mut blob = [0u8; 1 + MAX_SSID + 1 + MAX_PASS];
        let n = creds.serialize(&mut blob);
        let mut scratch = [0u8; SCRATCH];
        let mut map =
            MapStorage::<u8, _, _>::new(&mut self.flash, MapConfig::new(CREDS_RANGE), NoCache::new());
        map.store_item(&mut scratch, &CREDS_KEY, &&blob[..n])
            .await
            .map_err(|_| StorageError::Flash)
    }

    /// Erase all stored credentials (used by the BOOTSEL-hold factory reset).
    pub async fn clear(&mut self) -> Result<(), StorageError> {
        let mut map =
            MapStorage::<u8, _, _>::new(&mut self.flash, MapConfig::new(CREDS_RANGE), NoCache::new());
        map.erase_all().await.map_err(|_| StorageError::Flash)
    }
}
