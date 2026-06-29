#![no_std]

extern crate alloc;

use hal::mmio::{IoPort, Mmio};

pub type PciResult<T = ()> = Result<T, PciError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PciError {
    DeviceNotFound,
    InvalidVendor,
    BarInvalid,
    InvalidBusRange,
    ConfigAccessFailed,
}

#[derive(Debug, Clone, Copy)]
pub struct PciAddress {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciAddress {
    pub const fn new(bus: u8, device: u8, function: u8) -> Self {
        Self { bus, device, function }
    }

    fn to_config_addr(&self, offset: u8) -> u32 {
        0x8000_0000
            | (self.bus as u32) << 16
            | (self.device as u32) << 11
            | (self.function as u32) << 8
            | (offset as u32 & 0xFC)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PciDeviceInfo {
    pub address: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision: u8,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
    pub subsystem_vendor: u16,
    pub subsystem_id: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Bar {
    pub address: u64,
    pub size: u64,
    pub is_mmio: bool,
    pub is_prefetchable: bool,
}

pub struct PciBus {
    config_mode: PciConfigMode,
}

enum PciConfigMode {
    Io,
    Mmio(Mmio),
}

impl PciBus {
    pub fn new_io() -> Self {
        Self { config_mode: PciConfigMode::Io }
    }

    pub fn new_mmio(base: usize, size: usize) -> Self {
        Self { config_mode: PciConfigMode::Mmio(unsafe { Mmio::new(base, size) }) }
    }

    fn config_read32(&self, addr: PciAddress, offset: u8) -> u32 {
        match self.config_mode {
            PciConfigMode::Io => {
                let config_addr = addr.to_config_addr(offset);
                IoPort::new(0xCF8).outl(config_addr);
                IoPort::new(0xCFC).inl()
            }
            PciConfigMode::Mmio(mmio) => {
                let off = (addr.bus as usize) << 20
                    | (addr.device as usize) << 15
                    | (addr.function as usize) << 12
                    | (offset as usize);
                mmio.r32(off)
            }
        }
    }

    fn config_write32(&self, addr: PciAddress, offset: u8, val: u32) {
        match self.config_mode {
            PciConfigMode::Io => {
                let config_addr = addr.to_config_addr(offset);
                IoPort::new(0xCF8).outl(config_addr);
                IoPort::new(0xCFC).outl(val);
            }
            PciConfigMode::Mmio(mmio) => {
                let off = (addr.bus as usize) << 20
                    | (addr.device as usize) << 15
                    | (addr.function as usize) << 12
                    | (offset as usize);
                mmio.w32(off, val)
            }
        }
    }

    fn config_read16(&self, addr: PciAddress, offset: u8) -> u16 {
        let val = self.config_read32(addr, offset);
        let shift = (offset as u32 & 0x02) * 8;
        (val >> shift) as u16
    }

    fn config_read8(&self, addr: PciAddress, offset: u8) -> u8 {
        let val = self.config_read32(addr, offset);
        let shift = (offset as u32 & 0x03) * 8;
        (val >> shift) as u8
    }

    pub fn enumerate(&self) -> PciResult<alloc::vec::Vec<PciDeviceInfo>> {
        let mut devices = alloc::vec::Vec::new();

        for bus in 0..=255u8 {
            for device in 0..=31u8 {
                let addr = PciAddress::new(bus, device, 0);
                let vendor = self.config_read16(addr, 0x00);

                if vendor == 0xFFFF || vendor == 0 {
                    continue;
                }

                let header_type = self.config_read8(addr, 0x0E);
                let functions = if header_type & 0x80 != 0 { 8 } else { 1 };

                for function in 0..functions {
                    let addr = PciAddress::new(bus, device, function as u8);
                    let vendor = self.config_read16(addr, 0x00);
                    if vendor == 0xFFFF {
                        continue;
                    }

                    let device_id = self.config_read16(addr, 0x02);
                    let rev = self.config_read8(addr, 0x08);
                    let class = self.config_read8(addr, 0x0B);
                    let subclass = self.config_read8(addr, 0x0A);
                    let prog_if = self.config_read8(addr, 0x09);
                    let subsys_vendor = self.config_read16(addr, 0x2C);
                    let subsys_id = self.config_read16(addr, 0x2E);

                    devices.push(PciDeviceInfo {
                        address: addr,
                        vendor_id: vendor,
                        device_id,
                        revision: rev,
                        class,
                        subclass,
                        prog_if,
                        header_type,
                        subsystem_vendor: subsys_vendor,
                        subsystem_id: subsys_id,
                    });
                }
            }
        }

        Ok(devices)
    }

    pub fn read_bar(&self, addr: PciAddress, bar_index: u8) -> PciResult<Bar> {
        if bar_index > 5 {
            return Err(PciError::BarInvalid);
        }

        let bar_reg = 0x10 + bar_index * 4;
        let low = self.config_read32(addr, bar_reg);

        let is_mmio = low & 0x01 == 0;
        let is_prefetchable = if is_mmio { low & 0x08 != 0 } else { false };

        let address = if is_mmio {
            let addr_low = low & 0xFFFFFFF0;
            if bar_index == 5 {
                let hi = self.config_read32(addr, bar_reg + 4);
                (addr_low as u64) | ((hi as u64) << 32)
            } else {
                addr_low as u64
            }
        } else {
            (low & 0xFFFC) as u64
        };

        self.config_write32(addr, bar_reg, 0xFFFF_FFFF);
        let size_raw = self.config_read32(addr, bar_reg);
        self.config_write32(addr, bar_reg, low);

        let size = if is_mmio {
            let mask = size_raw & 0xFFFFFFF0;
            if mask == 0 { 0 } else { (!mask + 1) as u64 }
        } else {
            let mask = size_raw & 0xFFFC;
            if mask == 0 { 0 } else { (!mask + 1) as u64 }
        };

        Ok(Bar { address, size, is_mmio, is_prefetchable })
    }

    pub fn set_bus_master(&self, addr: PciAddress, enable: bool) {
        let cmd = self.config_read16(addr, 0x04);
        let cmd = if enable { cmd | 0x04 } else { cmd & !0x04 };
        self.config_write16(addr, 0x04, cmd);
    }

    pub fn read_config(&self, addr: PciAddress, offset: u8) -> u32 {
        self.config_read32(addr, offset)
    }

    pub fn write_config(&self, addr: PciAddress, offset: u8, val: u32) {
        self.config_write32(addr, offset, val);
    }

    fn config_write16(&self, addr: PciAddress, offset: u8, val: u16) {
        let shift = (offset as u32 & 0x02) * 8;
        let old = self.config_read32(addr, offset);
        let new = (old & !(0xFFFF << shift)) | ((val as u32) << shift);
        self.config_write32(addr, offset, new);
    }
}
