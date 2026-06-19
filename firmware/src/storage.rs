//! Persistent Wi-Fi credential storage.
//!
//! Stores the SSID + password using [`sequential_storage`] (CRC-checked, power-fail safe) over
//! esp-storage's raw flash, inside the `nvs` data partition's range. We do *not* use the esp-idf
//! NVS on-flash format — this no_std firmware just reuses that partition's otherwise-unused flash
//! region. esp-storage's flash is blocking, so we adapt it to the async API with `BlockingAsync`.
//!
//! KEEP IN SYNC: docs/wifi-onboarding.md documents the partition assumption.

use core::ops::Range;

use embassy_embedded_hal::adapter::BlockingAsync;
use esp_bootloader_esp_idf::partitions::{
    read_partition_table, DataPartitionSubType, PartitionType,
};
use esp_storage::FlashStorage;
use heapless::String;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{MapConfig, MapStorage};

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
    /// Could not read the partition table or locate the `nvs` partition.
    Init,
    /// A flash read/write/erase failed.
    Flash,
}

type Flash = BlockingAsync<FlashStorage<'static>>;

/// Owns the flash and the resolved credential storage range.
pub struct CredStore {
    flash: Flash,
    range: Range<u32>,
}

impl CredStore {
    /// Take the flash peripheral and resolve the `nvs` partition's flash range.
    pub fn new(flash: esp_hal::peripherals::FLASH<'static>) -> Result<Self, StorageError> {
        let mut fs = FlashStorage::new(flash);

        // Read just enough of the partition table sector to find `nvs`.
        let mut table = [0u8; 0x400];
        let range = {
            let pt = read_partition_table(&mut fs, &mut table).map_err(|_| StorageError::Init)?;
            let nvs = pt
                .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
                .map_err(|_| StorageError::Init)?
                .ok_or(StorageError::Init)?;
            let start = nvs.offset();
            start..start + nvs.len()
        };
        log::info!(
            "[storage] using nvs range {:#x}..{:#x}",
            range.start,
            range.end
        );

        Ok(Self {
            flash: BlockingAsync::new(fs),
            range,
        })
    }

    /// Load saved credentials, or `None` if unprovisioned (or on a recoverable read error).
    pub async fn load(&mut self) -> Option<Credentials> {
        let mut buf = [0u8; SCRATCH];
        let mut map = MapStorage::<u8, _, _>::new(
            &mut self.flash,
            MapConfig::new(self.range.clone()),
            NoCache::new(),
        );
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
        let mut map = MapStorage::<u8, _, _>::new(
            &mut self.flash,
            MapConfig::new(self.range.clone()),
            NoCache::new(),
        );
        map.store_item(&mut scratch, &CREDS_KEY, &&blob[..n])
            .await
            .map_err(|_| StorageError::Flash)
    }

    /// Erase all stored credentials (used by the BOOT-hold reset).
    pub async fn clear(&mut self) -> Result<(), StorageError> {
        let mut map = MapStorage::<u8, _, _>::new(
            &mut self.flash,
            MapConfig::new(self.range.clone()),
            NoCache::new(),
        );
        map.erase_all().await.map_err(|_| StorageError::Flash)
    }
}
