#![no_std]

use hal::mmio::Mmio;

fn phys_read8(addr: usize) -> u8 {
    let mmio = unsafe { Mmio::new(addr, 1) };
    mmio.r8(0)
}

fn phys_read32(addr: usize) -> u32 {
    let mmio = unsafe { Mmio::new(addr, 4) };
    mmio.r32(0)
}

fn phys_read64(addr: usize) -> u64 {
    let mmio = unsafe { Mmio::new(addr, 8) };
    mmio.r64(0)
}

fn phys_write8(addr: usize, val: u8) {
    let mmio = unsafe { Mmio::new(addr, 1) };
    mmio.w8(0, val)
}

fn phys_write16(addr: usize, val: u16) {
    let mmio = unsafe { Mmio::new(addr, 2) };
    mmio.w16(0, val)
}

pub type AcpiResult<T = ()> = Result<T, AcpiError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiError {
    NoRsdp,
    BadChecksum,
    TableNotFound,
    NotSupported,
    HwPlatformError,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct Rsdp {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    pub xsdt_address: u64,
    pub ext_checksum: u8,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct GenericAddress {
    pub address_space: u8,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum PowerState {
    G0Working,
    G1Sleeping,
    G2SoftOff,
    G3MechanicalOff,
}

#[derive(Debug, Clone, Copy)]
pub struct AcpiPmResource {
    pub pm1a_cnt_blk: u16,
    pub pm1b_cnt_blk: u16,
    pub pm1a_evt_blk: u16,
    pub pm1b_evt_blk: u16,
}

pub struct Acpi {
    rsdp: &'static Rsdp,
    rsdt: &'static [SdtHeader],
    xsdt_present: bool,
    pm_resource: Option<AcpiPmResource>,
    facp: Option<&'static SdtHeader>,
}

impl Acpi {
    pub fn new(rsdp_addr: usize) -> AcpiResult<Self> {
        let rsdp = unsafe { &*(rsdp_addr as *const Rsdp) };

        if &rsdp.signature != b"RSD PTR " {
            return Err(AcpiError::NoRsdp);
        }

        if rsdp.revision == 0 {
            let rsdt_addr = rsdp.rsdt_address as usize;
            let rsdt = unsafe { &*(rsdt_addr as *const SdtHeader) };
            let count = (rsdt.length as usize - core::mem::size_of::<SdtHeader>()) / 4;

            Ok(Self {
                rsdp,
                rsdt: unsafe { core::slice::from_raw_parts(rsdt_addr as *const SdtHeader, count + 1) },
                xsdt_present: false,
                pm_resource: None,
                facp: None,
            })
        } else {
            let xsdt_addr = rsdp.xsdt_address as usize;
            let xsdt = unsafe { &*(xsdt_addr as *const SdtHeader) };
            let count = (xsdt.length as usize - core::mem::size_of::<SdtHeader>()) / 8;

            let fake = unsafe { core::slice::from_raw_parts(xsdt_addr as *const SdtHeader, count + 1) };

            Ok(Self {
                rsdp,
                rsdt: fake,
                xsdt_present: true,
                pm_resource: None,
                facp: None,
            })
        }
    }

    pub fn find_table(&self, signature: &[u8; 4]) -> Option<usize> {
        let count = if self.xsdt_present {
            let xsdt = unsafe { &*self.rsdt.as_ptr() };
            (xsdt.length as usize - core::mem::size_of::<SdtHeader>()) / 8
        } else {
            let rsdt = unsafe { &*self.rsdt.as_ptr() };
            (rsdt.length as usize - core::mem::size_of::<SdtHeader>()) / 4
        };

        for i in 0..count {
            let addr = if self.xsdt_present {
                let ptrs = unsafe {
                    core::slice::from_raw_parts(
                        (self.rsdt.as_ptr() as usize + core::mem::size_of::<SdtHeader>()) as *const u64,
                        count,
                    )
                };
                ptrs[i] as usize
            } else {
                let ptrs = unsafe {
                    core::slice::from_raw_parts(
                        (self.rsdt.as_ptr() as usize + core::mem::size_of::<SdtHeader>()) as *const u32,
                        count,
                    )
                };
                ptrs[i] as usize
            };

            let hdr = unsafe { &*(addr as *const SdtHeader) };
            if &hdr.signature == signature {
                return Some(addr);
            }
        }

        None
    }

    pub fn load_facp(&mut self) -> AcpiResult {
        let addr = self.find_table(b"FACP").ok_or(AcpiError::TableNotFound)?;

        #[repr(C, packed)]
        struct Facp {
            hdr: SdtHeader,
            firmware_ctrl: u32,
            dsdt: u32,
            _reserved1: u8,
            preferred_pm_profile: u8,
            sci_int: u16,
            smi_cmd: u32,
            acpi_enable: u8,
            acpi_disable: u8,
            s4bios_req: u8,
            pstate_cnt: u8,
            pm1a_evt_blk: u32,
            pm1b_evt_blk: u32,
            pm1a_cnt_blk: u32,
            pm1b_cnt_blk: u32,
            pm2_cnt_blk: u32,
            pm_tmr_blk: u32,
            gpe0_blk: u32,
            gpe1_blk: u32,
            pm1_evt_len: u8,
            pm1_cnt_len: u8,
            pm2_cnt_len: u8,
            pm_tmr_len: u8,
            gpe0_blk_len: u8,
            gpe1_blk_len: u8,
            gpe1_base: u8,
            _cst_cnt: u8,
            plvl2_lat: u16,
            plvl3_lat: u16,
            flush_size: u16,
            flush_stride: u16,
            duty_offset: u8,
            duty_width: u8,
            day_alrm: u8,
            mon_alrm: u8,
            century: u8,
            iapc_boot_arch: u16,
            _reserved2: u8,
            flags: u32,
            reset_reg: GenericAddress,
            reset_value: u8,
            arm_boot_arch: u16,
            fadt_minor: u8,
            x_firmware_ctrl: u64,
            x_dsdt: u64,
            x_pm1a_evt_blk: GenericAddress,
            x_pm1b_evt_blk: GenericAddress,
            x_pm1a_cnt_blk: GenericAddress,
            x_pm1b_cnt_blk: GenericAddress,
            x_pm2_cnt_blk: GenericAddress,
            x_pm_tmr_blk: GenericAddress,
            x_gpe0_blk: GenericAddress,
            x_gpe1_blk: GenericAddress,
        }

        let facp = unsafe { &*(addr as *const Facp) };

        self.pm_resource = Some(AcpiPmResource {
            pm1a_cnt_blk: facp.pm1a_cnt_blk as u16,
            pm1b_cnt_blk: facp.pm1b_cnt_blk as u16,
            pm1a_evt_blk: facp.pm1a_evt_blk as u16,
            pm1b_evt_blk: facp.pm1b_evt_blk as u16,
        });

        self.facp = Some(unsafe { &*(addr as *const SdtHeader) });
        Ok(())
    }

    pub fn pm_resource(&self) -> Option<&AcpiPmResource> {
        self.pm_resource.as_ref()
    }

    pub fn sleep(&self, state: PowerState) -> AcpiResult {
        let pm = self.pm_resource.ok_or(AcpiError::NotSupported)?;

        match state {
            PowerState::G1Sleeping => {
                let slp_typa = 1u16;
                let slp_typb = 0u16;
                let val = (slp_typa << 10) | (slp_typb << 10) | (1 << 13);
                phys_write16(pm.pm1a_cnt_blk as usize, val);
                Ok(())
            }
            PowerState::G2SoftOff => {
                let slp_typa = 5u16;
                                let slp_typb = 5u16;
                let val = (slp_typa << 10) | (1 << 13);
                phys_write16(pm.pm1a_cnt_blk as usize, val);
                Ok(())


            }
            _ => Err(AcpiError::NotSupported),
        }
    }

    pub fn reboot(&self) -> AcpiResult {
        let addr = self.find_table(b"FACP").ok_or(AcpiError::TableNotFound)?;

        #[repr(C, packed)]
        struct FacpShort {
            hdr: SdtHeader,
            _pad: [u8; 116],
            reset_reg: GenericAddress,
            reset_value: u8,
        }

        let facp = unsafe { &*(addr as *const FacpShort) };
        let reg = &facp.reset_reg;

        if reg.address == 0 {
            return Err(AcpiError::NotSupported);
        }

        match reg.address_space {
            0 => {
                phys_write8(reg.address as usize, facp.reset_value);
            }
            1 => {
                phys_write8(reg.address as usize, facp.reset_value);
            }
            _ => return Err(AcpiError::NotSupported),
        }

        Ok(())
    }

    pub fn shutdown(&self) -> AcpiResult {
        let addr = self.find_table(b"FACP").ok_or(AcpiError::TableNotFound)?;

        let pm = self.pm_resource.ok_or(AcpiError::NotSupported)?;
        let slp_typa = 5u16;
        let val = (slp_typa << 10) | (1 << 13);
                phys_write16(pm.pm1a_cnt_blk as usize, val);

        Ok(())
    }
}
