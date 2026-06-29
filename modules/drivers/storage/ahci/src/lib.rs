#![no_std]

use core::ptr::{read_volatile, write_volatile, addr_of_mut};

pub type AhciResult<T = ()> = Result<T, AhciError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AhciError {
    NotFound,
    NoPorts,
    DeviceBusy,
    CommandFailed,
    Timeout,
    InvalidSector,
    DmaError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    None,
    Sata,
    Satapi,
    Semu,
}

#[repr(C, packed)]
pub struct HbaMemory {
    pub cap: u32,
    pub ghc: u32,
    pub is: u32,
    pub pi: u32,
    pub vs: u32,
    pub ccc_ctl: u32,
    pub ccc_pts: u32,
    pub em_loc: u32,
    pub em_ctl: u32,
    pub cap2: u32,
    pub bohc: u32,
    pub _reserved: [u32; 29],
    pub vendor: [u32; 24],
    pub ports: [HbaPort; 32],
}

#[repr(C, packed)]
pub struct HbaPort {
    pub clb: u32,
    pub clbu: u32,
    pub fb: u32,
    pub fbu: u32,
    pub is: u32,
    pub ie: u32,
    pub cmd: u32,
    pub _reserved0: u32,
    pub tfd: u32,
    pub sig: u32,
    pub ssts: u32,
    pub sctl: u32,
    pub serr: u32,
    pub sact: u32,
    pub ci: u32,
    pub sntf: u32,
    pub fbs: u32,
    pub _reserved1: [u32; 11],
    pub vendor: [u32; 4],
}

#[repr(C, packed)]
pub struct CommandHeader {
    pub opts: u16,
    pub prdtl: u16,
    pub prdbc: u32,
    pub ctba: u32,
    pub ctbau: u32,
    pub _reserved: [u32; 4],
}

#[repr(C, packed)]
pub struct CommandTable {
    pub cfis: [u8; 64],
    pub acmd: [u8; 16],
    pub _reserved: [u8; 48],
    pub prdt: [PrdEntry; 8],
}

#[repr(C, packed)]
pub struct PrdEntry {
    pub dba: u32,
    pub dbau: u32,
    pub _reserved: u32,
    pub dbc: u32,
}

pub struct AhciController {
    mmio_base: usize,
    port_count: u32,
    ports: [Option<AhciPort>; 32],
    initialized: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct AhciPort {
    pub id: usize,
    pub port_type: PortType,
    pub signature: u32,
    pub sector_count: u64,
}

const HBA_GHC_IE: u32 = 0x00000002;
const HBA_GHC_AE: u32 = 0x80000000;
const HBA_GHC_HR: u32 = 0x00000001;

const HBA_PXCMD_ST: u32 = 0x00000001;
const HBA_PXCMD_FRE: u32 = 0x00000010;
const HBA_PXCMD_FR: u32 = 0x00004000;
const HBA_PXCMD_CR: u32 = 0x00008000;

const SATA_SIG_ATA: u32 = 0x00000101;
const SATA_SIG_ATAPI: u32 = 0xEB140101;
const SATA_SIG_PM: u32 = 0x96690101;
const SATA_SIG_SEMB: u32 = 0xC33C0101;

impl AhciController {
    pub fn new(mmio_base: usize) -> Self {
        Self {
            mmio_base,
            port_count: 0,
            ports: [None; 32],
            initialized: false,
        }
    }

    fn hba(&self) -> &HbaMemory {
        unsafe { &*(self.mmio_base as *const HbaMemory) }
    }

    fn hba_mut(&mut self) -> &mut HbaMemory {
        unsafe { &mut *(self.mmio_base as *mut HbaMemory) }
    }

    fn port(&self, i: usize) -> &HbaPort {
        unsafe { &(*(self.mmio_base as *const HbaMemory)).ports[i] }
    }

    fn port_mut(&mut self, i: usize) -> &mut HbaPort {
        unsafe { &mut (*(self.mmio_base as *mut HbaMemory)).ports[i] }
    }

    pub fn init(&mut self) -> AhciResult {
        let hba_ptr = self.mmio_base as *mut HbaMemory;

        let cap = unsafe { read_volatile(addr_of_mut!((*hba_ptr).cap)) };
        if cap == 0xFFFFFFFF || cap == 0 {
            return Err(AhciError::NotFound);
        }

        let ghc = unsafe { read_volatile(addr_of_mut!((*hba_ptr).ghc)) };
        unsafe { write_volatile(addr_of_mut!((*hba_ptr).ghc), ghc | HBA_GHC_HR) };
        for _ in 0..10000 {
            if unsafe { read_volatile(addr_of_mut!((*hba_ptr).ghc)) } & HBA_GHC_HR == 0 {
                break;
            }
        }

        unsafe { write_volatile(addr_of_mut!((*hba_ptr).ghc), HBA_GHC_AE) };
        let ghc_val = unsafe { read_volatile(addr_of_mut!((*hba_ptr).ghc)) };
        unsafe { write_volatile(addr_of_mut!((*hba_ptr).ghc), ghc_val | HBA_GHC_IE) };

        let pi = unsafe { read_volatile(addr_of_mut!((*hba_ptr).pi)) };

        for i in 0..32 {
            if pi & (1 << i) == 0 {
                continue;
            }

            let port_ptr = self.mmio_base + core::mem::offset_of!(HbaMemory, ports) + i * core::mem::size_of::<HbaPort>();
            let cmd_addr = port_ptr + core::mem::offset_of!(HbaPort, cmd);
            let ssts_addr = port_ptr + core::mem::offset_of!(HbaPort, ssts);
            let sig_addr = port_ptr + core::mem::offset_of!(HbaPort, sig);

            let cmd = unsafe { read_volatile(cmd_addr as *const u32) };
            let ssts = unsafe { read_volatile(ssts_addr as *const u32) };

            if cmd & HBA_PXCMD_CR != 0 {
                unsafe { write_volatile(cmd_addr as *mut u32, cmd & !HBA_PXCMD_ST) };
                for _ in 0..1000 {
                    if unsafe { read_volatile(cmd_addr as *const u32) } & HBA_PXCMD_CR == 0 {
                        break;
                    }
                }
            }

            let ipm = (ssts >> 8) & 0x0F;
            let det = ssts & 0x0F;
            if det != 0x03 || ipm != 0x01 {
                continue;
            }

            let sig = unsafe { read_volatile(sig_addr as *const u32) };
            let port_type = match sig {
                SATA_SIG_ATA => PortType::Sata,
                SATA_SIG_ATAPI => PortType::Satapi,
                SATA_SIG_SEMB => PortType::Semu,
                _ => PortType::None,
            };

            self.ports[i] = Some(AhciPort {
                id: i,
                port_type,
                signature: sig,
                sector_count: self.identify_device(i).unwrap_or(0),
            });

            self.port_count += 1;
        }

        if self.port_count == 0 {
            return Err(AhciError::NoPorts);
        }

        self.initialized = true;
        Ok(())
    }

    fn identify_device(&self, port_no: usize) -> AhciResult<u64> {
        let _ = port_no;
        Err(AhciError::CommandFailed)
    }

    pub fn port_count(&self) -> u32 {
        self.port_count
    }

    pub fn get_port(&self, index: usize) -> Option<&AhciPort> {
        self.ports.get(index).and_then(|p| p.as_ref())
    }

    pub fn read_sectors(&self, port_no: usize, lba: u64, count: u16, buffer: &mut [u8]) -> AhciResult {
        let _ = (port_no, lba, count, buffer);
        Err(AhciError::CommandFailed)
    }

    pub fn write_sectors(&self, port_no: usize, lba: u64, count: u16, buffer: &[u8]) -> AhciResult {
        let _ = (port_no, lba, count, buffer);
        Err(AhciError::CommandFailed)
    }

    pub fn flush_cache(&self, port_no: usize) -> AhciResult {
        let _ = port_no;
        Ok(())
    }
}
