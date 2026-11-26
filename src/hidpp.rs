//! Minimal HID++ 2.0 client for Logitech receivers and mice on Windows.

use anyhow::{Context, Result, anyhow, bail};
use hidapi::{HidApi, HidDevice};
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

const LOGITECH_VID: u16 = 0x046d;
const HIDPP_USAGE_PAGE: u16 = 0xff00;
const USAGE_SHORT: u16 = 0x0001;
const USAGE_LONG: u16 = 0x0002;
const REPORT_SHORT: u8 = 0x10;
const REPORT_LONG: u8 = 0x11;
const LONG_LEN: usize = 20;
const SW_ID: u8 = 0x0b;
const TIMEOUT: Duration = Duration::from_millis(800);

/// HID++ 2.0 feature IDs used here.
pub mod feature {
    pub const ROOT: u16 = 0x0000;
    pub const DEVICE_NAME: u16 = 0x0005;
    pub const ADJUSTABLE_DPI: u16 = 0x2201;
    pub const EXT_ADJUSTABLE_DPI: u16 = 0x2202;
    pub const REPORT_RATE: u16 = 0x8060;
    pub const EXT_REPORT_RATE: u16 = 0x8061;
    pub const ONBOARD_PROFILES: u16 = 0x8100;
}

/// Rates of feature 0x8061, indexed by bit: 125 Hz << bit.
pub const EXT_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Onboard,
    Host,
    Other(u8),
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Onboard => write!(f, "onboard"),
            Mode::Host => write!(f, "host"),
            Mode::Other(v) => write!(f, "unknown ({v})"),
        }
    }
}

/// An error reported by the device itself.
#[derive(Debug)]
pub struct DeviceError(pub u8);

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            1 => "unknown",
            2 => "invalid argument",
            3 => "out of range",
            4 => "hardware error",
            5 => "not allowed",
            6 => "invalid feature index",
            7 => "invalid function",
            8 => "busy",
            9 => "unsupported",
            _ => "other",
        };
        write!(f, "device error {} ({name})", self.0)
    }
}
impl std::error::Error for DeviceError {}

/// The HID++ collections of one receiver or wired mouse.
struct Channel {
    long: HidDevice,
    short: Option<HidDevice>,
    product_id: u16,
    product: String,
}

pub struct Device {
    ch: Channel,
    pub index: u8,
    features: HashMap<u16, u8>,
    /// DPI read from the onboard profile before we took over, restored in host mode.
    onboard_dpi: Option<Dpi>,
}

/// Sensor resolution. `y` and `lod` (lift-off distance) only exist on 0x2202 mice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dpi {
    pub x: u16,
    pub y: u16,
    pub lod: Option<u8>,
}

impl fmt::Display for Dpi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.x == self.y { write!(f, "{}", self.x) } else { write!(f, "{}x{}", self.x, self.y) }
    }
}

fn open_channels(api: &HidApi) -> Vec<Channel> {
    let infos: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == LOGITECH_VID && d.usage_page() == HIDPP_USAGE_PAGE)
        .collect();
    let mut out = Vec::new();
    for long in infos.iter().filter(|d| d.usage() == USAGE_LONG) {
        let Ok(long_dev) = api.open_path(long.path()) else { continue };
        let short_dev = infos
            .iter()
            .find(|d| {
                d.usage() == USAGE_SHORT
                    && d.product_id() == long.product_id()
                    && d.interface_number() == long.interface_number()
            })
            .and_then(|d| api.open_path(d.path()).ok());
        out.push(Channel {
            long: long_dev,
            short: short_dev,
            product_id: long.product_id(),
            product: long.product_string().unwrap_or("?").to_string(),
        });
    }
    out
}

impl Device {
    /// Finds the first reachable HID++ 2.0 device that can change its report rate.
    pub fn open() -> Result<Device> {
        let api = HidApi::new().context("HID API init failed")?;
        let mut seen = Vec::new();
        for ch in open_channels(&api) {
            let label = format!("{} [{:04x}]", ch.product, ch.product_id);
            let mut dev = Device { ch, index: 0, features: HashMap::new(), onboard_dpi: None };
            for index in [1u8, 2, 3, 4, 5, 6, 0xff] {
                dev.index = index;
                dev.features.clear();
                match dev.protocol() {
                    Ok((major, _)) if major >= 2 => {}
                    _ => continue,
                }
                if dev.has(feature::EXT_REPORT_RATE)? || dev.has(feature::REPORT_RATE)? {
                    return Ok(dev);
                }
            }
            seen.push(label);
        }
        if seen.is_empty() {
            bail!("no Logitech HID++ receiver or mouse found");
        }
        bail!("no reachable mouse with report-rate control on: {} (is the mouse awake?)", seen.join(", "))
    }

    pub fn receiver(&self) -> String {
        format!("{} [{:04x}]", self.ch.product, self.ch.product_id)
    }

    fn request(&self, feat_index: u8, function: u8, params: &[u8]) -> Result<Vec<u8>> {
        let mut buf = [0u8; LONG_LEN];
        buf[0] = REPORT_LONG;
        buf[1] = self.index;
        buf[2] = feat_index;
        buf[3] = (function << 4) | SW_ID;
        buf[4..4 + params.len()].copy_from_slice(params);

        let mut scratch = [0u8; 64];
        while self.ch.long.read_timeout(&mut scratch, 0)? > 0 {}
        if let Some(short) = &self.ch.short {
            while short.read_timeout(&mut scratch, 0)? > 0 {}
        }
        self.ch.long.write(&buf).context("HID write failed")?;

        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            let mut r = [0u8; 64];
            let n = self.ch.long.read_timeout(&mut r, 15)?;
            if n >= 7 && r[0] == REPORT_LONG && r[1] == self.index {
                if r[2] == 0xff && r[3] == feat_index && r[4] == buf[3] {
                    return Err(DeviceError(r[5]).into());
                }
                if r[2] == feat_index && r[3] == buf[3] {
                    return Ok(r[4..n].to_vec());
                }
            }
            // HID++ 1.0 error from the receiver: the paired device didn't answer.
            if let Some(short) = &self.ch.short {
                let n = short.read_timeout(&mut r, 0)?;
                if n >= 6 && r[0] == REPORT_SHORT && r[1] == self.index && r[2] == 0x8f {
                    bail!("device {} unreachable (receiver error {:#04x})", self.index, r[5]);
                }
            }
        }
        bail!("no answer from device {} (asleep or off?)", self.index)
    }

    /// HID++ protocol version (root.ping).
    pub fn protocol(&self) -> Result<(u8, u8)> {
        let r = self.request(0x00, 1, &[0, 0, 0x5a])?;
        Ok((r[0], r[1]))
    }

    fn feature_index(&mut self, id: u16) -> Result<Option<u8>> {
        if let Some(&i) = self.features.get(&id) {
            return Ok((i != 0).then_some(i));
        }
        let r = self.request(0x00, 0, &id.to_be_bytes())?;
        self.features.insert(id, r[0]);
        Ok((r[0] != 0).then_some(r[0]))
    }

    /// Raw feature call for diagnostics (`logirate raw`).
    pub fn raw(&mut self, id: u16, function: u8, params: &[u8]) -> Result<Vec<u8>> {
        self.call(id, function, params)
    }

    pub fn has(&mut self, id: u16) -> Result<bool> {
        Ok(self.feature_index(id)?.is_some())
    }

    fn call(&mut self, id: u16, function: u8, params: &[u8]) -> Result<Vec<u8>> {
        let idx = self.feature_index(id)?.ok_or_else(|| anyhow!("feature {id:#06x} not supported"))?;
        self.request(idx, function, params)
    }

    pub fn name(&mut self) -> Result<String> {
        let len = self.call(feature::DEVICE_NAME, 0, &[])?[0] as usize;
        let mut name = Vec::new();
        while name.len() < len {
            let chunk = self.call(feature::DEVICE_NAME, 1, &[name.len() as u8])?;
            let take = (len - name.len()).min(chunk.len());
            name.extend_from_slice(&chunk[..take]);
        }
        Ok(String::from_utf8_lossy(&name).trim_end_matches('\0').to_string())
    }

    // ---- report rate ----

    pub fn supported_rates(&mut self) -> Result<Vec<u32>> {
        if self.has(feature::EXT_REPORT_RATE)? {
            let r = self.call(feature::EXT_REPORT_RATE, 1, &[])?;
            let mask = u16::from_be_bytes([r[0], r[1]]);
            return Ok((0..EXT_RATES.len()).filter(|b| mask >> b & 1 == 1).map(|b| EXT_RATES[b]).collect());
        }
        let mask = self.call(feature::REPORT_RATE, 0, &[])?[0];
        Ok((0..8u32).filter(|b| mask >> b & 1 == 1).map(|b| 1000 / (b + 1)).collect())
    }

    /// 0x8061 reads the rate per connection type: 0 = wired, 1 = wireless.
    /// Through a receiver we're wireless; a directly cabled mouse answers on index 0xff.
    fn connection_type(&self) -> u8 {
        if self.index == 0xff { 0 } else { 1 }
    }

    pub fn rate(&mut self) -> Result<u32> {
        if self.has(feature::EXT_REPORT_RATE)? {
            let conn = self.connection_type();
            let i = self.call(feature::EXT_REPORT_RATE, 2, &[conn])?[0] as usize;
            return EXT_RATES.get(i).copied().ok_or_else(|| anyhow!("unknown rate index {i}"));
        }
        let ms = self.call(feature::REPORT_RATE, 1, &[])?[0] as u32;
        Ok(1000 / ms.max(1))
    }

    fn write_rate(&mut self, hz: u32) -> Result<()> {
        if self.has(feature::EXT_REPORT_RATE)? {
            let i = EXT_RATES.iter().position(|&r| r == hz).ok_or_else(|| anyhow!("{hz} Hz is not a valid rate"))?;
            self.call(feature::EXT_REPORT_RATE, 3, &[i as u8])?;
        } else {
            if 1000 % hz != 0 {
                bail!("{hz} Hz is not a valid rate for this mouse");
            }
            self.call(feature::REPORT_RATE, 2, &[(1000 / hz) as u8])?;
        }
        Ok(())
    }

    // ---- onboard profiles / DPI ----

    pub fn mode(&mut self) -> Result<Option<Mode>> {
        if !self.has(feature::ONBOARD_PROFILES)? {
            return Ok(None);
        }
        Ok(Some(match self.call(feature::ONBOARD_PROFILES, 2, &[])?[0] {
            1 => Mode::Onboard,
            2 => Mode::Host,
            v => Mode::Other(v),
        }))
    }

    fn set_mode(&mut self, mode: Mode) -> Result<()> {
        let v = match mode {
            Mode::Onboard => 1,
            Mode::Host => 2,
            Mode::Other(v) => v,
        };
        self.call(feature::ONBOARD_PROFILES, 1, &[v])?;
        Ok(())
    }

    pub fn has_dpi(&mut self) -> Result<bool> {
        Ok(self.has(feature::EXT_ADJUSTABLE_DPI)? || self.has(feature::ADJUSTABLE_DPI)?)
    }

    /// Raw reply of 0x2202 getSensorDpi, for diagnostics.
    pub fn dpi_raw(&mut self) -> Result<Vec<u8>> {
        self.call(feature::EXT_ADJUSTABLE_DPI, 5, &[0])
    }

    pub fn dpi(&mut self) -> Result<Dpi> {
        if self.has(feature::EXT_ADJUSTABLE_DPI)? {
            // [sensor, x(2), default x(2), y(2), default y(2), lod]
            let r = self.call(feature::EXT_ADJUSTABLE_DPI, 5, &[0])?;
            let x = u16::from_be_bytes([r[1], r[2]]);
            let y = u16::from_be_bytes([r[5], r[6]]);
            return Ok(Dpi { x, y: if y == 0 { x } else { y }, lod: Some(r[9]) });
        }
        let r = self.call(feature::ADJUSTABLE_DPI, 2, &[0])?;
        let x = u16::from_be_bytes([r[1], r[2]]);
        Ok(Dpi { x, y: x, lod: None })
    }

    fn set_dpi(&mut self, dpi: Dpi) -> Result<()> {
        if !(50..=44000).contains(&dpi.x) || !(50..=44000).contains(&dpi.y) {
            bail!("refusing implausible DPI {dpi}");
        }
        let [xh, xl] = dpi.x.to_be_bytes();
        if self.has(feature::EXT_ADJUSTABLE_DPI)? {
            let [yh, yl] = dpi.y.to_be_bytes();
            self.call(feature::EXT_ADJUSTABLE_DPI, 6, &[0, xh, xl, yh, yl, dpi.lod.unwrap_or(0)])?;
        } else {
            self.call(feature::ADJUSTABLE_DPI, 3, &[0, xh, xl])?;
        }
        Ok(())
    }

    // ---- high level ----

    /// Hands control back to the mouse's own onboard profile.
    pub fn use_onboard(&mut self) -> Result<()> {
        if let Some(mode) = self.mode()? {
            if mode != Mode::Onboard {
                self.set_mode(Mode::Onboard)?;
            }
        }
        Ok(())
    }

    /// Switches to host mode and sets `hz`, plus `dpi` if given (else the onboard profile's DPI).
    pub fn set_host(&mut self, hz: u32, dpi: Option<u16>) -> Result<()> {
        let supported = self.supported_rates()?;
        if !supported.contains(&hz) {
            bail!("{hz} Hz not supported; this mouse offers {supported:?}");
        }
        if self.mode()? == Some(Mode::Onboard) {
            self.onboard_dpi = if self.has_dpi()? { Some(self.dpi()?) } else { None };
            self.set_mode(Mode::Host)?;
        }
        if let Some(want) = self.wanted_dpi(dpi) {
            if self.dpi()? != want {
                self.set_dpi(want).with_context(|| format!("setting {want} DPI"))?;
            }
        }
        self.write_rate(hz)?;
        let now = self.rate()?;
        if now != hz {
            bail!("asked for {hz} Hz but the mouse reports {now} Hz");
        }
        Ok(())
    }

    /// True if the mouse is still in host mode with these settings
    /// (a power cycle drops it back to its onboard profile).
    pub fn is_at(&mut self, hz: u32, dpi: Option<u16>) -> Result<bool> {
        if !matches!(self.mode()?, None | Some(Mode::Host)) || self.rate()? != hz {
            return Ok(false);
        }
        match self.wanted_dpi(dpi) {
            Some(want) => Ok(self.dpi()? == want),
            None => Ok(true),
        }
    }

    /// The DPI host mode should use: the rule's, else the onboard profile's.
    fn wanted_dpi(&self, rule_dpi: Option<u16>) -> Option<Dpi> {
        match (rule_dpi, self.onboard_dpi) {
            (Some(d), base) => Some(Dpi { x: d, y: d, lod: base.and_then(|b| b.lod) }),
            (None, base) => base,
        }
    }
}
