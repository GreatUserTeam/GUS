#![no_std]

use hal::mmio::Mmio;

pub type EthResult<T = ()> = Result<T, EthError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EthError {
    NotFound,
    LinkDown,
    TxBufferFull,
    RxBufferEmpty,
    InvalidMac,
    Timeout,
}

pub const MAC_LEN: usize = 6;

pub type MacAddress = [u8; MAC_LEN];

#[derive(Debug, Clone, Copy)]
pub struct EthDeviceInfo {
    pub vendor: u16,
    pub device: u16,
    pub mac: MacAddress,
    pub speed_mbps: u32,
    pub full_duplex: bool,
    pub mtu: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct RxPacket {
    pub data: [u8; 2048],
    pub len: u16,
}

#[repr(C, packed)]
pub struct TxDescriptor {
    pub addr: u64,
    pub len: u16,
    pub flags: u16,
    pub next: u64,
}

#[repr(C, packed)]
pub struct RxDescriptor {
    pub addr: u64,
    pub len: u16,
    pub flags: u16,
    pub next: u64,
}

pub struct EthernetController {
    mmio: Mmio,
    mac: MacAddress,
    link_up: bool,
    speed: u32,
    full_duplex: bool,
    rx_desc_base: usize,
    tx_desc_base: usize,
    rx_cur: u16,
    tx_cur: u16,
    rx_entries: u16,
    tx_entries: u16,
    initialized: bool,
}

impl EthernetController {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            mac: [0u8; 6],
            link_up: false,
            speed: 0,
            full_duplex: false,
            rx_desc_base: 0,
            tx_desc_base: 0,
            rx_cur: 0,
            tx_cur: 0,
            rx_entries: 64,
            tx_entries: 32,
            initialized: false,
        }
    }

    pub fn init(&mut self) -> EthResult {
        let vendor_id = self.mmio.r16(0x00);
        let _device_id = self.mmio.r16(0x02);
        if vendor_id == 0xFFFF || vendor_id == 0 {
            return Err(EthError::NotFound);
        }

        self.read_mac();

        self.reset();
        self.init_rx();
        self.init_tx();

        self.mmio.w32(0x04, 0x06);
        self.mmio.w32(0x40, 0x01);

        self.initialized = true;
        Ok(())
    }

    fn read_mac(&mut self) {
        let mac_low = self.mmio.r32(0x00);
        let mac_high = self.mmio.r16(0x04);

        self.mac = [
            mac_low as u8,
            (mac_low >> 8) as u8,
            (mac_low >> 16) as u8,
            (mac_low >> 24) as u8,
            mac_high as u8,
            (mac_high >> 8) as u8,
        ];
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }

    fn reset(&self) {
        self.mmio.w32(0x08, 0x01);
        for _ in 0..1000 {
            if self.mmio.r8(0x08) & 0x01 == 0 {
                break;
            }
        }
    }

    fn init_rx(&mut self) {
        self.mmio.w32(0x44, self.rx_desc_base as u32);
        self.mmio.w16(0x48, self.rx_entries);
        self.mmio.w32(0x4C, 0x01);
    }

    fn init_tx(&mut self) {
        self.mmio.w32(0x50, self.tx_desc_base as u32);
        self.mmio.w16(0x54, self.tx_entries);
        self.mmio.w32(0x58, 0x01);
    }

    pub fn link_status(&self) -> bool {
        self.mmio.r8(0x14) & 0x01 != 0
    }

    pub fn speed(&self) -> u32 {
        let sts = self.mmio.r8(0x14);
        match sts >> 2 & 0x03 {
            0 => 10,
            1 => 100,
            2 => 1000,
            _ => 0,
        }
    }

    pub fn send_packet(&mut self, data: &[u8]) -> EthResult {
        if !self.link_up {
            return Err(EthError::LinkDown);
        }

        let desc_addr = self.tx_desc_base + (self.tx_cur as usize) * 16;
        let buf_addr = self.mmio.r32(desc_addr) as u64 | (self.mmio.r32(desc_addr + 8) as u64) << 32;

        let buf = unsafe { core::slice::from_raw_parts_mut(buf_addr as *mut u8, data.len()) };
        buf.copy_from_slice(data);

        self.mmio.w32(desc_addr + 4, data.len() as u32 | 0x0100_0000);

        self.tx_cur = (self.tx_cur + 1) % self.tx_entries;
        self.mmio.w32(0x5C, 0x01);

        Ok(())
    }

    pub fn receive_packet(&mut self) -> Option<RxPacket> {
        let desc_addr = self.rx_desc_base + (self.rx_cur as usize) * 16;
        let flags = self.mmio.r16(desc_addr + 12);

        if flags & 0x8000 == 0 {
            return None;
        }

        let buf_addr = self.mmio.r32(desc_addr) as u64 | (self.mmio.r32(desc_addr + 8) as u64) << 32;
        let pkt_len = (flags & 0x3FFF) as usize;

        let mut packet = RxPacket { data: [0u8; 2048], len: pkt_len as u16 };
        let buf = unsafe { core::slice::from_raw_parts(buf_addr as *const u8, pkt_len) };
        packet.data[..pkt_len].copy_from_slice(buf);

        self.mmio.w16(desc_addr + 12, 0x0000);
        self.rx_cur = (self.rx_cur + 1) % self.rx_entries;

        Some(packet)
    }
}
