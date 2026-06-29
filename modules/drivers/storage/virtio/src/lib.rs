#![no_std]

use hal::mmio::Mmio;

pub type VirtioResult<T = ()> = Result<T, VirtioError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioError {
    NotFound,
    InitFailed,
    QueueFull,
    QueueEmpty,
    DeviceError,
    InvalidFeature,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioDeviceType {
    Network,
    Block,
    Console,
    Entropy,
    MemoryBalloon,
    IoMemory,
    Rpmsg,
    Scsi,
    Audio,
    Gpu,
    Input,
    Vsock,
    Unknown,
}

impl VirtioDeviceType {
    pub fn from_id(id: u32) -> Self {
        match id {
            1 => Self::Network,
            2 => Self::Block,
            3 => Self::Console,
            4 => Self::Entropy,
            5 => Self::MemoryBalloon,
            6 => Self::IoMemory,
            7 => Self::Rpmsg,
            8 => Self::Scsi,
            9 => Self::Audio,
            16 => Self::Gpu,
            18 => Self::Input,
            19 => Self::Vsock,
            _ => Self::Unknown,
        }
    }
}

#[repr(C, packed)]
pub struct VirtioPciCap {
    pub cap_vndr: u8,
    pub cap_next: u8,
    pub cap_len: u8,
    pub cfg_type: u8,
    pub bar: u8,
    pub _padding: [u8; 3],
    pub offset: u32,
    pub length: u32,
}

const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;

const VRING_DESC_F_NEXT: u16 = 1;
const VRING_DESC_F_WRITE: u16 = 2;

#[repr(C, packed)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C, packed)]
pub struct VirtqAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; 256],
    pub used_event: u16,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C, packed)]
pub struct VirtqUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VirtqUsedElem; 256],
    pub avail_event: u16,
}

#[repr(C, packed)]
pub struct VirtioBlkReq {
    pub type_: u32,
    pub reserved: u32,
    pub sector: u64,
}

#[repr(C, packed)]
pub struct VirtioBlkResp {
    pub status: u8,
}

pub struct VirtioQueue {
    pub desc: &'static mut [VirtqDesc],
    pub avail: &'static mut VirtqAvail,
    pub used: &'static mut VirtqUsed,
    pub index: u16,
    pub size: u16,
    pub free_head: u16,
    pub num_free: u16,
}

impl VirtioQueue {
    pub fn new(index: u16, size: u16, desc: &'static mut [VirtqDesc], avail: &'static mut VirtqAvail, used: &'static mut VirtqUsed) -> Self {
        Self {
            desc,
            avail,
            used,
            index,
            size,
            free_head: 0,
            num_free: size,
        }
    }

    pub fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let desc_idx = self.free_head;
        self.free_head = self.desc[desc_idx as usize].next;
        self.num_free -= 1;
        Some(desc_idx)
    }

    pub fn free_desc(&mut self, desc_idx: u16) {
        self.desc[desc_idx as usize].next = self.free_head;
        self.free_head = desc_idx;
        self.num_free += 1;
    }

    pub fn add_buf(&mut self, desc_idx: u16) {
        let idx = self.avail.idx as u16 % self.size;
        self.avail.ring[idx as usize] = desc_idx;
        self.avail.idx = self.avail.idx.wrapping_add(1);
    }

    pub fn notify(&self, mmio: &Mmio, queue_notify_off: u32) {
        mmio.w16(queue_notify_off as usize, self.index);
    }

    pub fn used_idx(&self) -> u16 {
        self.used.idx
    }

    pub fn get_used_elem(&self, idx: u16) -> VirtqUsedElem {
        self.used.ring[idx as usize % self.size as usize]
    }
}

pub struct VirtioTransport {
    mmio: Mmio,
    device_type: VirtioDeviceType,
    device_features: u64,
    negotiated: u64,
    queue_notify_off: u32,
    queue: Option<VirtioQueue>,
    initialized: bool,
}

impl VirtioTransport {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            device_type: VirtioDeviceType::Unknown,
            device_features: 0,
            negotiated: 0,
            queue_notify_off: 0,
            queue: None,
            initialized: false,
        }
    }



    pub fn init(&mut self) -> VirtioResult {
        let magic = self.mmio.r32(0x00);
        if magic != 0x74726976 {
            return Err(VirtioError::NotFound);
        }

        let version = self.mmio.r32(0x04);
        if version != 2 && version != 1 {
            return Err(VirtioError::NotFound);
        }

        let device_id_val = self.mmio.r32(0x08);
        self.device_type = VirtioDeviceType::from_id(device_id_val);
        let _vendor_id = self.mmio.r32(0x0C);

        let device_features = self.mmio.r64(0x10);
        self.device_features = device_features;

        self.mmio.w32(0x20, VIRTIO_F_VERSION_1 as u32);
        let negotiated = self.mmio.r64(0x20);
        self.negotiated = negotiated;

        let device_status = self.mmio.r8(0x22);
        self.mmio.w8(0x22, device_status | 0x01);
        self.mmio.w8(0x22, device_status | 0x03);
        self.mmio.w8(0x22, device_status | 0x07);
        self.mmio.w8(0x22, device_status | 0x0F);

        for _ in 0..1000 {
            if self.mmio.r8(0x22) & 0x0F == 0x0F {
                break;
            }
        }

        self.mmio.w8(0x22, self.mmio.r8(0x22) | 0x10);
        for _ in 0..10000 {
            if self.mmio.r8(0x22) & 0x02 == 0 {
                break;
            }
        }

        self.queue_notify_off = self.mmio.r32(0x30);

        self.initialized = true;
        Ok(())
    }

    fn setup_queue(&mut self, index: u16) -> Option<&mut VirtioQueue> {
        self.mmio.w16(0x2A, index);
        self.mmio.w16(0x2C, 0);
        let size = self.mmio.r16(0x2C);
        if size == 0 {
            return None;
        }

        self.mmio.w16(0x2A, index);
        self.mmio.w16(0x2C, 0);

        let desc_addr = self.alloc_dma(size as usize * 16) as u64;
        let avail_addr = self.alloc_dma(6 + size as usize * 2) as u64;
        let used_addr = self.alloc_dma(6 + size as usize * 8) as u64;

        self.mmio.w64(0x2E, desc_addr);
        self.mmio.w64(0x36, avail_addr);
        self.mmio.w64(0x3E, used_addr);
        self.mmio.w16(0x44, 0x01);

        None
    }

    fn alloc_dma(&self, _size: usize) -> *mut u8 {
        core::ptr::null_mut()
    }

    pub fn block_read(&mut self, sector: u64, buf: &mut [u8]) -> VirtioResult {
        let _ = (sector, buf);
        Err(VirtioError::DeviceError)
    }

    pub fn block_write(&mut self, sector: u64, buf: &[u8]) -> VirtioResult {
        let _ = (sector, buf);
        Err(VirtioError::DeviceError)
    }

    pub fn device_type(&self) -> VirtioDeviceType {
        self.device_type
    }

    pub fn features(&self) -> u64 {
        self.device_features
    }
}
