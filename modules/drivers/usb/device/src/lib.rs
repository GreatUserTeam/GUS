#![no_std]

use hal::mmio::Mmio;

pub type UdcResult<T = ()> = Result<T, UdcError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdcError {
    NotFound,
    InitFailed,
    EndpointError,
    Stall,
    BufferOverrun,
    Disconnected,
}

const MAX_ENDPOINTS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDir {
    Out,
    In,
    Bidirectional,
}

#[derive(Debug, Clone, Copy)]
pub struct EndpointConfig {
    pub number: u8,
    pub dir: EndpointDir,
    pub max_packet: u16,
    pub transfer_type: TransferType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbDeviceDescriptor {
    pub bcd_usb: u16,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    pub max_packet_size: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub manufacturer: u8,
    pub product: u8,
    pub serial: u8,
    pub num_configs: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum SetupRequest {
    GetStatus,
    ClearFeature,
    SetFeature,
    SetAddress,
    GetDescriptor,
    SetDescriptor,
    GetConfiguration,
    SetConfiguration,
    GetInterface,
    SetInterface,
    SynchFrame,
    Unknown(u8),
}

impl SetupRequest {
    pub fn from_bm(bm: u8) -> Self {
        match bm {
            0x00 => Self::GetStatus,
            0x01 => Self::ClearFeature,
            0x03 => Self::SetFeature,
            0x05 => Self::SetAddress,
            0x06 => Self::GetDescriptor,
            0x07 => Self::SetDescriptor,
            0x08 => Self::GetConfiguration,
            0x09 => Self::SetConfiguration,
            0x0A => Self::GetInterface,
            0x0B => Self::SetInterface,
            0x0C => Self::SynchFrame,
            x => Self::Unknown(x),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SetupPacket {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
}

pub struct UsbDeviceController {
    mmio: Mmio,
    endpoints: [Option<EndpointDir>; MAX_ENDPOINTS],
    address: u8,
    configured: bool,
    ep0_in_progress: bool,
    initialized: bool,
}

impl UsbDeviceController {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            endpoints: [None; MAX_ENDPOINTS],
            address: 0,
            configured: false,
            ep0_in_progress: false,
            initialized: false,
        }
    }

    pub fn init(&mut self, desc: &UsbDeviceDescriptor) -> UdcResult {
        let vendor = self.mmio.r16(0x00);
        if vendor == 0xFFFF || vendor == 0 {
            return Err(UdcError::NotFound);
        }

        self.mmio.w32(0x04, 0x01);
        for _ in 0..1000 {
            if self.mmio.r32(0x04) & 0x01 != 0 {
                break;
            }
        }

        self.set_device_desc(desc);

        self.mmio.w8(0x08, desc.max_packet_size);

        self.endpoints[0] = Some(EndpointDir::Bidirectional);

        self.configured = false;
        self.initialized = true;
        Ok(())
    }

    fn set_device_desc(&self, desc: &UsbDeviceDescriptor) {
        self.mmio.w16(0x10, desc.bcd_usb);
        self.mmio.w8(0x12, desc.device_class);
        self.mmio.w8(0x13, desc.device_subclass);
        self.mmio.w8(0x14, desc.device_protocol);
        self.mmio.w16(0x16, desc.vendor_id);
        self.mmio.w16(0x18, desc.product_id);
        self.mmio.w16(0x1A, desc.bcd_device);
    }

    pub fn configure_endpoint(&mut self, config: &EndpointConfig) -> UdcResult {
        let idx = config.number as usize;
        if idx >= MAX_ENDPOINTS {
            return Err(UdcError::EndpointError);
        }

        let ep_reg = 0x20 + idx * 8;
        self.mmio.w8(ep_reg, config.number);
        self.mmio.w8(ep_reg + 1, config.dir as u8);
        self.mmio.w16(ep_reg + 2, config.max_packet);
        self.mmio.w8(ep_reg + 4, config.transfer_type as u8);
        self.mmio.w8(ep_reg + 5, 0x01);

        self.endpoints[idx] = Some(config.dir);
        Ok(())
    }

    pub fn read_setup_packet(&self) -> Option<SetupPacket> {
        let sts = self.mmio.r32(0x00);
        if sts & 0x02 == 0 {
            return None;
        }

        Some(SetupPacket {
            bm_request_type: self.mmio.r8(0x80),
            b_request: self.mmio.r8(0x81),
            w_value: self.mmio.r16(0x82),
            w_index: self.mmio.r16(0x84),
            w_length: self.mmio.r16(0x86),
        })
    }

    pub fn send_data(&self, ep: u8, data: &[u8]) -> UdcResult {
        let ep_idx = ep as usize;
        if ep_idx >= MAX_ENDPOINTS || self.endpoints[ep_idx].is_none() {
            return Err(UdcError::EndpointError);
        }

        let fifo_reg = 0x100 + ep_idx * 0x20;
        let len = data.len().min(64);
        for i in 0..len {
            self.mmio.w8(fifo_reg + i, data[i]);
        }
        self.mmio.w16(0x04 + ep_idx as usize, len as u16);

        Ok(())
    }

    pub fn receive_data(&self, ep: u8, buf: &mut [u8]) -> UdcResult<u16> {
        let ep_idx = ep as usize;
        if ep_idx >= MAX_ENDPOINTS || self.endpoints[ep_idx].is_none() {
            return Err(UdcError::EndpointError);
        }

        let sts = self.mmio.r16(0x04 + ep_idx);
        if sts & 0x01 == 0 {
            return Err(UdcError::EndpointError);
        }

        let fifo_reg = 0x100 + ep_idx * 0x20;
        let len = self.mmio.r8(fifo_reg) as usize;
        let copy_len = len.min(buf.len());
        for i in 0..copy_len {
            buf[i] = self.mmio.r8(fifo_reg + i);
        }

        Ok(copy_len as u16)
    }

    pub fn stall_ep(&self, ep: u8) {
        let ep_idx = ep as usize;
        if ep_idx < MAX_ENDPOINTS {
            self.mmio.w8(0x20 + ep_idx * 8 + 6, 0x01);
        }
    }

    pub fn set_address(&mut self, addr: u8) {
        self.mmio.w8(0x08, addr);
        self.address = addr;
    }

    pub fn set_configured(&mut self, configured: bool) {
        self.configured = configured;
        if configured {
            self.mmio.w32(0x0C, 0x01);
        } else {
            self.mmio.w32(0x0C, 0x00);
        }
    }
}
