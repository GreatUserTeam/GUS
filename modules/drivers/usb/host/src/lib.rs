#![no_std]

use hal::mmio::Mmio;

pub type HostResult<T = ()> = Result<T, HostError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    NotFound,
    InitFailed,
    TransferFailed,
    Stall,
    Babble,
    NoBandwidth,
    Timeout,
    PortDisabled,
    NotImplemented,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostControllerType {
    Uhci,
    Ohci,
    Ehci,
    Xhci,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbDevice {
    pub address: u8,
    pub port: u8,
    pub speed: UsbSpeed,
    pub vendor_id: u16,
    pub product_id: u16,
    pub class: u8,
    pub subclass: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,
    Full,
    High,
    Super,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbSetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

impl UsbSetupPacket {
    pub fn get_descriptor(desc_type: u8, index: u8, lang_id: u16) -> Self {
        Self {
            bm_request_type: 0x80,
            b_request: 0x06,
            w_value: ((desc_type as u16) << 8) | index as u16,
            w_index: lang_id,
            w_length: 0xFF,
        }
    }

    pub fn set_address(addr: u8) -> Self {
        Self {
            bm_request_type: 0x00,
            b_request: 0x05,
            w_value: addr as u16,
            w_index: 0,
            w_length: 0,
        }
    }

    pub fn set_configuration(config: u8) -> Self {
        Self {
            bm_request_type: 0x00,
            b_request: 0x09,
            w_value: config as u16,
            w_index: 0,
            w_length: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TransferDescriptor {
    pub addr: u64,
    pub len: u32,
    pub ioc: u8,
    pub toggle: u8,
    pub status: u32,
}

pub struct UsbHostController {
    mmio: Mmio,
    hc_type: HostControllerType,
    ports: u8,
    devices: [Option<UsbDevice>; 128],
    next_addr: u8,
    initialized: bool,
}

impl UsbHostController {
    pub fn new(base: usize, size: usize, hc_type: HostControllerType) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            hc_type,
            ports: 0,
            devices: [None; 128],
            next_addr: 1,
            initialized: false,
        }
    }

    pub fn init(&mut self) -> HostResult {
        let ver = self.mmio.r16(0x00);
        if ver == 0xFFFF || ver == 0 {
            return Err(HostError::NotFound);
        }

        match self.hc_type {
            HostControllerType::Ehci => {
                self.mmio.w32(0x20, 0x01);
                for _ in 0..1000 {
                    if self.mmio.r32(0x20) & 0x01 == 0 {
                        break;
                    }
                }

                self.mmio.w32(0x24, 0x00);
                self.mmio.w32(0x28, 0x00);
                self.mmio.w32(0x34, 0x01);

                let hcc_params = self.mmio.r32(0x08);
                self.ports = ((hcc_params >> 24) & 0x0F) as u8;
            }
            HostControllerType::Xhci => {
                self.mmio.w32(0x04, self.mmio.r32(0x04) | 0x01);
                for _ in 0..10000 {
                    if self.mmio.r32(0x04) & 0x01 != 0 {
                        break;
                    }
                }

                let hcsparams1 = self.mmio.r32(0x04);
                self.ports = (hcsparams1 & 0xFF) as u8;
            }
            HostControllerType::Uhci | HostControllerType::Ohci => {
                self.ports = 2;
            }
        }

        self.initialized = true;
        Ok(())
    }

    pub fn port_count(&self) -> u8 {
        self.ports
    }

    pub fn port_status(&self, port: u8) -> u32 {
        match self.hc_type {
            HostControllerType::Ehci => self.mmio.r32(0x44 + (port as usize) * 4),
            HostControllerType::Xhci => self.mmio.r32(0x400 + (port as usize) * 0x10),
            _ => 0,
        }
    }

    pub fn port_speed(&self, port: u8) -> UsbSpeed {
        let sts = self.port_status(port);
        match self.hc_type {
            HostControllerType::Ehci => {
                match (sts >> 26) & 0x03 {
                    0 => UsbSpeed::Full,
                    1 => UsbSpeed::Low,
                    2 => UsbSpeed::High,
                    _ => UsbSpeed::Full,
                }
            }
            HostControllerType::Xhci => {
                match (sts >> 10) & 0x0F {
                    1 => UsbSpeed::Low,
                    2 => UsbSpeed::Full,
                    3 => UsbSpeed::High,
                    4 => UsbSpeed::Super,
                    _ => UsbSpeed::Full,
                }
            }
            _ => UsbSpeed::Full,
        }
    }

    pub fn reset_port(&self, port: u8) -> HostResult {
        match self.hc_type {
            HostControllerType::Ehci => {
                let addr = 0x44 + (port as usize) * 4;
                self.mmio.w32(addr, self.mmio.r32(addr) | 0x0100);
                for _ in 0..10000 {
                    let sts = self.mmio.r32(addr);
                    if sts & 0x0100 == 0 {
                        break;
                    }
                }
                Ok(())
            }
            HostControllerType::Xhci => {
                let addr = 0x400 + (port as usize) * 0x10;
                self.mmio.w32(addr, self.mmio.r32(addr) | 0x10);
                for _ in 0..10000 {
                    let sts = self.mmio.r32(addr);
                    if sts & 0x01 != 0 {
                        return Ok(());
                    }
                }
                Err(HostError::Timeout)
            }
            _ => Err(HostError::NotImplemented),
        }
    }

    pub fn control_transfer(&self, dev_addr: u8, setup: &UsbSetupPacket, data: &mut [u8]) -> HostResult<u16> {
        let _ = (dev_addr, setup, data);
        Err(HostError::TransferFailed)
    }

    pub fn enumerate_device(&mut self, port: u8) -> HostResult<u8> {
        let addr = self.next_addr;
        self.next_addr += 1;

        let setup = UsbSetupPacket::set_address(addr);
        self.control_transfer(0, &setup, &mut [])?;

        let mut buf = [0u8; 18];
        let desc_req = UsbSetupPacket::get_descriptor(1, 0, 0);
        let len = self.control_transfer(addr, &desc_req, &mut buf)?;

        if len < 18 {
            return Err(HostError::TransferFailed);
        }

        let device = UsbDevice {
            address: addr,
            port,
            speed: self.port_speed(port),
            vendor_id: u16::from_le_bytes([buf[8], buf[9]]),
            product_id: u16::from_le_bytes([buf[10], buf[11]]),
            class: buf[4],
            subclass: buf[5],
        };

        let idx = addr as usize;
        if idx < self.devices.len() {
            self.devices[idx] = Some(device);
        }

        Ok(addr)
    }

    pub fn device(&self, addr: u8) -> Option<&UsbDevice> {
        self.devices.get(addr as usize).and_then(|d| d.as_ref())
    }
}
