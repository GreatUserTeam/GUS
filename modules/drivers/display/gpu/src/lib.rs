#![no_std]

use hal::mmio::Mmio;

pub type GpuResult<T = ()> = Result<T, GpuError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuError {
    NotFound,
    InitFailed,
    NoMemory,
    InvalidMode,
    CommandQueueFull,
    Timeout,
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
    pub pitch: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum GpuVendor {
    Bochs,
    Virtio,
    Intel,
    Amd,
    Nvidia,
    Generic,
}

#[derive(Debug)]
pub struct GpuDevice {
    vendor: GpuVendor,
    mmio: Mmio,
    vram_base: usize,
    vram_size: usize,
    current_mode: Option<DisplayMode>,
    initialized: bool,
}

impl GpuDevice {
    pub fn new(vendor: GpuVendor, mmio: Mmio, vram_base: usize, vram_size: usize) -> Self {
        Self {
            vendor,
            mmio,
            vram_base,
            vram_size,
            current_mode: None,
            initialized: false,
        }
    }

    pub fn vendor(&self) -> GpuVendor {
        self.vendor
    }

    pub fn vram_base(&self) -> usize {
        self.vram_base
    }

    pub fn vram_size(&self) -> usize {
        self.vram_size
    }

    pub fn current_mode(&self) -> Option<&DisplayMode> {
        self.current_mode.as_ref()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn init(&mut self) -> GpuResult {
        if self.initialized {
            return Ok(());
        }

        let vendor_id = self.mmio.r32(0x00);
        let _device_id = self.mmio.r32(0x04);

        if vendor_id == 0xFFFF || vendor_id == 0 {
            return Err(GpuError::NotFound);
        }

        self.initialized = true;
        Ok(())
    }

    pub fn set_mode(&mut self, mode: &DisplayMode) -> GpuResult {
        if !self.initialized {
            return Err(GpuError::InitFailed);
        }

        let bytes_per_pixel = mode.bpp as u32 / 8;
        let pitch = mode.width * bytes_per_pixel;
        let fb_size = pitch * mode.height;

        if fb_size > self.vram_size as u32 {
            return Err(GpuError::NoMemory);
        }

        let mode_stored = DisplayMode {
            width: mode.width,
            height: mode.height,
            bpp: mode.bpp,
            pitch,
            refresh_hz: mode.refresh_hz,
        };

        self.mmio.w32(0x08, mode.width);
        self.mmio.w32(0x0C, mode.height);
        self.mmio.w32(0x10, mode.bpp as u32);
        self.mmio.w32(0x14, pitch);
        self.mmio.w32(0x18, self.vram_base as u32);
        self.mmio.w32(0x1C, 0x01);

        self.current_mode = Some(mode_stored);
        Ok(())
    }

    pub fn present(&self) -> GpuResult {
        if self.current_mode.is_none() {
            return Err(GpuError::InvalidMode);
        }

        self.mmio.w32(0x20, 0x01);
        Ok(())
    }

    pub fn wait_vblank(&self) -> GpuResult {
        let timeout = 100_000;
        let mut elapsed = 0;

        while elapsed < timeout {
            let status = self.mmio.r32(0x24);
            if status & 0x01 != 0 {
                self.mmio.w32(0x24, 0x01);
                return Ok(());
            }
            elapsed += 1;
        }

        Err(GpuError::Timeout)
    }

    pub fn edid(&self) -> Option<[u8; 128]> {
        let mut edid = [0u8; 128];
        let base = 0x100;

        if self.mmio.r32(base) == 0x00 {
            return None;
        }

        for i in 0..128 {
            edid[i] = self.mmio.r32((base + i as usize) & !0x03) as u8;
        }

        if edid[0] != 0x00 || edid[1] != 0xFF {
            return None;
        }

        Some(edid)
    }

    pub fn get_vram_slice(&self) -> Option<&[u8]> {
        if self.current_mode.is_none() {
            return None;
        }
        let mode = self.current_mode.as_ref().unwrap();
        let size = (mode.pitch * mode.height) as usize;
        unsafe { Some(core::slice::from_raw_parts(self.vram_base as *const u8, size)) }
    }

    pub fn get_vram_slice_mut(&mut self) -> Option<&mut [u8]> {
        if self.current_mode.is_none() {
            return None;
        }
        let mode = self.current_mode.as_ref().unwrap();
        let size = (mode.pitch * mode.height) as usize;
        unsafe { Some(core::slice::from_raw_parts_mut(self.vram_base as *mut u8, size)) }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GpuFramebuffer<'a> {
    fb_mmio: Mmio,
    device: &'a GpuDevice,
}

impl<'a> GpuFramebuffer<'a> {
    pub fn new(device: &'a GpuDevice) -> Option<Self> {
        if device.current_mode.is_none() {
            return None;
        }
        let fb_mmio = unsafe { Mmio::new(device.vram_base, device.vram_size) };
        Some(Self { fb_mmio, device })
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        let mode = self.device.current_mode.as_ref().unwrap();
        let bpp = mode.bpp as u32 / 8;

        if x >= mode.width || y >= mode.height {
            return;
        }

        let offset = (y * mode.pitch + x * bpp) as usize;

        self.fb_mmio.w8(offset, b);
        self.fb_mmio.w8(offset + 1, g);
        self.fb_mmio.w8(offset + 2, r);
        if bpp >= 4 {
            self.fb_mmio.w8(offset + 3, a);
        }
    }

    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        let mode = self.device.current_mode.as_ref().unwrap();
        let _fb_size = (mode.pitch * mode.height) as usize;
        let bpp = mode.bpp as u32 / 8;

        for y in 0..mode.height {
            for x in 0..mode.width {
                let offset = (y * mode.pitch + x * bpp) as usize;
                self.fb_mmio.w8(offset, b);
                self.fb_mmio.w8(offset + 1, g);
                self.fb_mmio.w8(offset + 2, r);
            }
        }
    }
}

pub fn probe(pci_vendor: u16, pci_device: u16, mmio: Mmio, vram_base: usize, vram_size: usize) -> Option<GpuDevice> {
    let vendor = match (pci_vendor, pci_device) {
        (0x1234, 0x1111) => GpuVendor::Bochs,
        (0x1AF4, 0x1050) => GpuVendor::Virtio,
        (0x8086, _) => GpuVendor::Intel,
        (0x1002, _) => GpuVendor::Amd,
        (0x10DE, _) => GpuVendor::Nvidia,
        _ => GpuVendor::Generic,
    };

    let mut device = GpuDevice::new(vendor, mmio, vram_base, vram_size);
    device.init().ok()?;
    Some(device)
}
