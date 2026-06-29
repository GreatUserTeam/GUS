#![no_std]

use hal::mmio::Mmio;

pub type WifiResult<T = ()> = Result<T, WifiError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiError {
    NotFound,
    InitFailed,
    ScanFailed,
    AuthFailed,
    AssocFailed,
    HandshakeFailed,
    Disconnected,
    TxFailed,
}

#[derive(Debug, Clone, Copy)]
pub enum WifiSecurity {
    Open,
    Wep,
    WpaPsk,
    Wpa2Psk,
    Wpa3Sae,
}

#[derive(Debug, Clone, Copy)]
pub enum WifiChannelWidth {
    MHz20,
    MHz40,
    MHz80,
    MHz160,
}

#[derive(Debug, Clone, Copy)]
pub struct Ssid(pub [u8; 32]);

impl Ssid {
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut ssid = [0u8; 32];
        let len = b.len().min(32);
        ssid[..len].copy_from_slice(&b[..len]);
        Self(ssid)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WifiAccessPoint {
    pub ssid: Ssid,
    pub bssid: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    pub security: WifiSecurity,
    pub max_rate_mbps: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct WifiConfig {
    pub ssid: Ssid,
    pub password: [u8; 64],
    pub password_len: u8,
    pub security: WifiSecurity,
}

pub struct WifiController {
    mmio: Mmio,
    initialized: bool,
    associated: bool,
    current_bssid: [u8; 6],
    channel: u8,
}

impl WifiController {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            initialized: false,
            associated: false,
            current_bssid: [0u8; 6],
            channel: 0,
        }
    }

    pub fn init(&mut self) -> WifiResult {
        let vendor = self.mmio.r16(0x00);
        if vendor == 0xFFFF || vendor == 0 {
            return Err(WifiError::NotFound);
        }

        self.mmio.w32(0x04, 0x01);
        for _ in 0..10000 {
            if self.mmio.r32(0x04) & 0x01 != 0 {
                break;
            }
        }

        let fw_status = self.mmio.r32(0x08);
        if fw_status != 0xFFFFFFFF && fw_status != 0 {
            self.load_firmware();
        }

        self.initialized = true;
        Ok(())
    }

    fn load_firmware(&self) {
        self.mmio.w32(0x0C, 0x00);
        self.mmio.w32(0x08, 0x01);
    }

    pub fn scan(&self) -> WifiResult {
        if !self.initialized {
            return Err(WifiError::InitFailed);
        }

        self.mmio.w32(0x10, 0x01);
        for _ in 0..50000 {
            if self.mmio.r32(0x10) & 0x01 == 0 {
                return Ok(());
            }
        }

        Err(WifiError::ScanFailed)
    }

    pub fn scan_results(&self, results: &mut [WifiAccessPoint]) -> usize {
        let count = self.mmio.r8(0x14) as usize;
        let max = results.len().min(count);

        for i in 0..max {
            let base = 0x100 + i * 48;

            let mut ssid = [0u8; 32];
            for j in 0..32 {
                ssid[j] = self.mmio.r8(base + j);
            }

            let mut bssid = [0u8; 6];
            for j in 0..6 {
                bssid[j] = self.mmio.r8(base + 32 + j);
            }

            results[i] = WifiAccessPoint {
                ssid: Ssid(ssid),
                bssid,
                channel: self.mmio.r8(base + 38),
                rssi: self.mmio.r8(base + 39) as i8,
                security: match self.mmio.r8(base + 40) {
                    0 => WifiSecurity::Open,
                    1 => WifiSecurity::Wep,
                    2 => WifiSecurity::WpaPsk,
                    3 => WifiSecurity::Wpa2Psk,
                    _ => WifiSecurity::Wpa2Psk,
                },
                max_rate_mbps: self.mmio.r16(base + 42),
            };
        }

        max
    }

    pub fn connect(&mut self, config: &WifiConfig) -> WifiResult {
        let base = 0x200;

        for i in 0..32 {
            self.mmio.w8(base + i, config.ssid.0[i]);
        }
        self.mmio.w8(base + 32, config.ssid.0.iter().position(|&c| c == 0).unwrap_or(32) as u8);

        for i in 0..config.password_len as usize {
            self.mmio.w8(base + 33 + i, config.password[i]);
        }
        self.mmio.w8(base + 33 + config.password_len as usize, config.password_len);
        self.mmio.w8(base + 100, config.security as u8);

        self.mmio.w32(0x18, 0x01);

        for _ in 0..100000 {
            let sts = self.mmio.r32(0x1C);
            if sts & 0x01 != 0 {
                for i in 0..6 {
                    self.current_bssid[i] = self.mmio.r8(base + 64 + i);
                }
                self.channel = self.mmio.r8(base + 70);
                self.associated = true;
                return Ok(());
            }
            if sts & 0x02 != 0 {
                break;
            }
        }

        Err(WifiError::AuthFailed)
    }

    pub fn disconnect(&mut self) -> WifiResult {
        self.mmio.w32(0x20, 0x01);
        self.associated = false;
        Ok(())
    }

    pub fn send_frame(&self, data: &[u8]) -> WifiResult {
        if !self.associated {
            return Err(WifiError::Disconnected);
        }

        let base = 0x300;
        let len = data.len().min(1500);
        for i in 0..len {
            self.mmio.w8(base + i, data[i]);
        }
        self.mmio.w16(base + 1500, len as u16);
        self.mmio.w32(0x24, 0x01);

        Ok(())
    }

    pub fn receive_frame(&self, buf: &mut [u8]) -> Option<u16> {
        let sts = self.mmio.r32(0x28);
        if sts & 0x01 == 0 {
            return None;
        }

        let len = self.mmio.r16(0x2C) as usize;
        let base = 0x400;
        let copy_len = len.min(buf.len());
        for i in 0..copy_len {
            buf[i] = self.mmio.r8(base + i);
        }

        self.mmio.w32(0x28, 0x01);
        Some(copy_len as u16)
    }
}
