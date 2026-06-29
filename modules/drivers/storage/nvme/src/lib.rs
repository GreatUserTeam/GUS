#![no_std]

use core::ptr::read_volatile;
use hal::mmio::Mmio;

pub type NvmeResult<T = ()> = Result<T, NvmeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvmeError {
    NotFound,
    InitFailed,
    QueueFull,
    CommandFailed,
    Timeout,
    InvalidNamespace,
    DmaError,
}

#[repr(C, packed)]
pub struct NvmeRegisters {
    pub cap: u64,
    pub vs: u32,
    pub intms: u32,
    pub intmc: u32,
    pub cc: u32,
    pub _rsvd1: u32,
    pub csts: u32,
    pub _rsvd2: u32,
    pub aqa: u32,
    pub asq: u64,
    pub acq: u64,
}

#[repr(C, packed)]
pub struct SubmissionQueueEntry {
    pub cdw0: u32,
    pub nsid: u32,
    pub _rsvd: [u64; 2],
    pub mptr: u64,
    pub prp1: u64,
    pub prp2: u64,
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
}

#[repr(C, packed)]
pub struct CompletionQueueEntry {
    pub cdw0: u32,
    pub cdw1: u32,
    pub sqhd: u16,
    pub sqid: u16,
    pub cid: u16,
    pub status: u16,
}

const NVME_CAP_CSS_NVM: u64 = 1 << 37;
const NVME_CC_EN: u32 = 0x0001;
const NVME_CC_CSS_NVM: u32 = 0x0000;
const NVME_CC_MPS_SHIFT: u32 = 7;
const NVME_CC_IOSQES_SHIFT: u32 = 16;
const NVME_CC_IOCQES_SHIFT: u32 = 20;
const NVME_CC_AMS_RR: u32 = 0x0000;
const NVME_CC_SHN_NONE: u32 = 0x0000;

const NVME_CSTS_RDY: u32 = 0x0001;
const NVME_CSTS_CFS: u32 = 0x0002;

const NVME_OPC_ADMIN_CREATE_IO_SQ: u8 = 0x01;
const NVME_OPC_ADMIN_CREATE_IO_CQ: u8 = 0x05;
const NVME_OPC_ADMIN_IDENTIFY: u8 = 0x06;

const NVME_QUEUE_SIZE: u16 = 64;

pub struct NvmeController {
    mmio: Mmio,
    admin_sq: *mut SubmissionQueueEntry,
    admin_cq: *mut CompletionQueueEntry,
    sq_tail: u16,
    cq_head: u16,
    qid: u16,
    ns_count: u32,
    initialized: bool,
    page_size: u32,
}

impl NvmeController {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            admin_sq: core::ptr::null_mut(),
            admin_cq: core::ptr::null_mut(),
            sq_tail: 0,
            cq_head: 0,
            qid: 0,
            ns_count: 0,
            initialized: false,
            page_size: 4096,
        }
    }

    fn regs(&self) -> &NvmeRegisters {
        unsafe { &*(self.mmio.base() as *const NvmeRegisters) }
    }

    pub fn init(&mut self, sq_mem: &'static mut [SubmissionQueueEntry; 64], cq_mem: &'static mut [CompletionQueueEntry; 64]) -> NvmeResult {
        let cap = self.mmio.r64(0x00);
        if cap == 0 || cap == 0xFFFF_FFFF_FFFF_FFFF {
            return Err(NvmeError::NotFound);
        }

        let cq_addr = cq_mem.as_ptr() as u64;
        let sq_addr = sq_mem.as_ptr() as u64;

        self.admin_sq = sq_mem.as_mut_ptr();
        self.admin_cq = cq_mem.as_mut_ptr();

        let aqa = (NVME_QUEUE_SIZE as u32) << 16 | NVME_QUEUE_SIZE as u32;
        self.mmio.w32(0x14, aqa);
        self.mmio.w64(0x18, cq_addr);
        self.mmio.w64(0x20, sq_addr);

        let cc = NVME_CC_EN
            | NVME_CC_CSS_NVM
            | (6 << NVME_CC_MPS_SHIFT)
            | (6 << NVME_CC_IOSQES_SHIFT)
            | (4 << NVME_CC_IOCQES_SHIFT);
        self.mmio.w32(0x08, cc);

        for _ in 0..10000 {
            let csts = self.mmio.r32(0x0C);
            if csts & NVME_CSTS_RDY != 0 {
                break;
            }
            if csts & NVME_CSTS_CFS != 0 {
                return Err(NvmeError::InitFailed);
            }
        }

        if self.mmio.r32(0x0C) & NVME_CSTS_RDY == 0 {
            return Err(NvmeError::InitFailed);
        }

        let identify = SubmissionQueueEntry {
            cdw0: NVME_OPC_ADMIN_IDENTIFY as u32,
            nsid: 0,
            prp1: 0,
            cdw10: 1,
            ..SubmissionQueueEntry::default()
        };

        self.submit_command(&identify)?;

        self.initialized = true;
        Ok(())
    }

    pub fn submit_command(&mut self, cmd: &SubmissionQueueEntry) -> NvmeResult {
        let slot = self.sq_tail as usize % NVME_QUEUE_SIZE as usize;
        unsafe {
            core::ptr::write_volatile(
                self.admin_sq.add(slot),
                SubmissionQueueEntry {
                    cdw0: cmd.cdw0,
                    nsid: cmd.nsid,
                    _rsvd: cmd._rsvd,
                    mptr: cmd.mptr,
                    prp1: cmd.prp1,
                    prp2: cmd.prp2,
                    cdw10: cmd.cdw10,
                    cdw11: cmd.cdw11,
                    cdw12: cmd.cdw12,
                    cdw13: cmd.cdw13,
                    cdw14: cmd.cdw14,
                    cdw15: cmd.cdw15,
                },
            );
        }

        self.sq_tail = (self.sq_tail + 1) % NVME_QUEUE_SIZE;
        self.mmio.w32(0x24, self.sq_tail as u32);

        for _ in 0..100000 {
            let cq_slot = self.cq_head as usize % NVME_QUEUE_SIZE as usize;
            let phase_ptr = unsafe { self.admin_cq.add(cq_slot) as *const u16 };
            let phase = unsafe { read_volatile(phase_ptr.add(3)) };
            if phase & 0x01 != 0 {
                let status = ((phase >> 1) & 0x7F) as u8;
                self.cq_head = (self.cq_head + 1) % NVME_QUEUE_SIZE;
                self.mmio.w32(0x10, self.cq_head as u32);

                if status != 0 {
                    return Err(NvmeError::CommandFailed);
                }
                return Ok(());
            }
        }

        Err(NvmeError::Timeout)
    }

    pub fn create_io_cq(&mut self, cq_id: u16, size: u16, addr: u64) -> NvmeResult {
        let mut cmd = SubmissionQueueEntry::default();
        cmd.cdw0 = (NVME_OPC_ADMIN_CREATE_IO_CQ as u32) | (7 << 16);
        cmd.prp1 = addr;
        cmd.cdw10 = (cq_id as u32) | ((size as u32 - 1) << 16);
        cmd.cdw11 = 0x0003;
        self.submit_command(&cmd)
    }

    pub fn create_io_sq(&mut self, sq_id: u16, size: u16, addr: u64, cq_id: u16) -> NvmeResult {
        let mut cmd = SubmissionQueueEntry::default();
        cmd.cdw0 = (NVME_OPC_ADMIN_CREATE_IO_SQ as u32) | (7 << 16);
        cmd.prp1 = addr;
        cmd.cdw10 = (sq_id as u32) | ((size as u32 - 1) << 16);
        cmd.cdw11 = (cq_id as u32) | (1 << 16);
        self.submit_command(&cmd)
    }

    pub fn namespace_count(&self) -> u32 {
        self.ns_count
    }

    pub fn read_blocks(&self, nsid: u32, lba: u64, count: u16, buf: u64) -> NvmeResult {
        let mut cmd = SubmissionQueueEntry::default();
        cmd.cdw0 = 0x02;
        cmd.nsid = nsid;
        cmd.prp1 = buf;
        cmd.cdw10 = lba as u32;
        cmd.cdw11 = (lba >> 32) as u32;
        cmd.cdw12 = count as u32 - 1;
        let _ = cmd;
        Err(NvmeError::CommandFailed)
    }

    pub fn write_blocks(&self, nsid: u32, lba: u64, count: u16, buf: u64) -> NvmeResult {
        let mut cmd = SubmissionQueueEntry::default();
        cmd.cdw0 = 0x01;
        cmd.nsid = nsid;
        cmd.prp1 = buf;
        cmd.cdw10 = lba as u32;
        cmd.cdw11 = (lba >> 32) as u32;
        cmd.cdw12 = count as u32 - 1;
        let _ = cmd;
        Err(NvmeError::CommandFailed)
    }
}

impl Default for SubmissionQueueEntry {
    fn default() -> Self {
        Self {
            cdw0: 0,
            nsid: 0,
            _rsvd: [0u64; 2],
            mptr: 0,
            prp1: 0,
            prp2: 0,
            cdw10: 0,
            cdw11: 0,
            cdw12: 0,
            cdw13: 0,
            cdw14: 0,
            cdw15: 0,
        }
    }
}
