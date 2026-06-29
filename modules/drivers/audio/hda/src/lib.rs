#![no_std]
#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

pub type HdaResult<T = ()> = Result<T, HdaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdaError {
    NotFound,
    NoCodecsFound,
    InitFailed,
    StreamInUse,
    InvalidFormat,
    DmaError,
    Timeout,
    CorbFull,
    RirbEmpty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdaStreamFormat {
    pub channels: u8,
    pub bits: u8,
    pub rate: u32,
}

impl HdaStreamFormat {
    pub fn to_hda_tag(&self) -> u16 {
        let rate_val = match self.rate {
            8000 => 0x00,
            11025 => 0x10,
            16000 => 0x20,
            22050 => 0x30,
            32000 => 0x40,
            44100 => 0x50,
            48000 => 0x60,
            88200 => 0x70,
            96000 => 0x80,
            192000 => 0x90,
            _ => 0x60,
        };

        let bits_val = match self.bits {
            8 => 0x00,
            16 => 0x10,
            20 => 0x20,
            24 => 0x30,
            32 => 0x40,
            _ => 0x10,
        };

        let channels_val = if self.channels == 0 { 0 } else { (self.channels - 1) & 0x07 };

        (1 << 14) | rate_val as u16 | bits_val as u16 | channels_val as u16
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HdaCodec {
    pub cad: u8,
    pub vendor_id: u32,
    pub revision: u32,
    pub nodes: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDir {
    Output,
    Input,
    Bidirectional,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HdaBdlEntry {
    pub address: u64,
    pub length: u32,
    pub ioc: u32,
}

const HDA_GCTL: usize = 0x0068;
const HDA_GSTS: usize = 0x0069;
const HDA_WAKEEN: usize = 0x006C;
const HDA_STATESTS: usize = 0x006E;
const HDA_INTCTL: usize = 0x0070;
const HDA_INTSTS: usize = 0x0074;

const HDA_CORB_BASE: usize = 0x0040;
const HDA_CORB_WP: usize = 0x0048;
const HDA_CORB_RP: usize = 0x004A;
const HDA_CORB_SIZE: usize = 0x004C;
const HDA_CORB_CTRL: usize = 0x004E;

const HDA_RIRB_BASE: usize = 0x0050;
const HDA_RIRB_WP: usize = 0x0058;
const HDA_RIRB_INTR: usize = 0x005A;
const HDA_RIRB_SIZE: usize = 0x005C;
const HDA_RIRB_CTRL: usize = 0x005E;

const SDS_CTRL: usize = 0x00;
const SDS_STS: usize = 0x02;
const SDS_LPIB: usize = 0x04;
const SDS_CBL: usize = 0x08;
const SDS_LVI: usize = 0x0C;
const SDS_BDPL: usize = 0x18;
const SDS_BDPU: usize = 0x1C;

const CORB_ENTRIES: usize = 256;
const RIRB_ENTRIES: usize = 256;

const SD_OFFSET: usize = 0x0080;
const SD_STRIDE: usize = 0x0020;

pub struct HdaController {
    mmio_base: usize,
    corb_ptr: *mut u32,
    rirb_ptr: *mut u32,
    corb_wp: u8,
    rirb_rp: u8,
    codecs: [Option<HdaCodec>; 16],
    initialized: bool,
}

impl HdaController {
    pub fn new(mmio_base: usize) -> Self {
        Self {
            mmio_base,
            corb_ptr: core::ptr::null_mut(),
            rirb_ptr: core::ptr::null_mut(),
            corb_wp: 0,
            rirb_rp: 0,
            codecs: [None; 16],
            initialized: false,
        }
    }

    fn r32(&self, off: usize) -> u32 {
        unsafe { read_volatile((self.mmio_base + off) as *const u32) }
    }

    fn w32(&self, off: usize, val: u32) {
        unsafe { write_volatile((self.mmio_base + off) as *mut u32, val) }
    }

    fn r16(&self, off: usize) -> u16 {
        unsafe { read_volatile((self.mmio_base + off) as *const u16) }
    }

    fn w16(&self, off: usize, val: u16) {
        unsafe { write_volatile((self.mmio_base + off) as *mut u16, val) }
    }

    fn r8(&self, off: usize) -> u8 {
        unsafe { read_volatile((self.mmio_base + off) as *const u8) }
    }

    fn w8(&self, off: usize, val: u8) {
        unsafe { write_volatile((self.mmio_base + off) as *mut u8, val) }
    }

    pub fn init(&mut self, corb_mem: &'static mut [u32; CORB_ENTRIES], rirb_mem: &'static mut [u32; RIRB_ENTRIES * 2]) -> HdaResult {
        let gctl = self.r32(HDA_GCTL);
        self.w32(HDA_GCTL, gctl | 0x0001);

        for _ in 0..1000 {
            if self.r8(HDA_GSTS) & 0x01 != 0 {
                break;
            }
        }
        if self.r8(HDA_GSTS) & 0x01 == 0 {
            return Err(HdaError::InitFailed);
        }

        self.corb_ptr = corb_mem.as_mut_ptr();
        self.rirb_ptr = rirb_mem.as_mut_ptr();

        self.w32(HDA_CORB_BASE, corb_mem.as_ptr() as u32);
        self.w32(HDA_CORB_BASE + 4, 0);
        self.w16(HDA_CORB_SIZE, 0x00 << 4);
        self.w16(HDA_CORB_CTRL, 0x0002);
        self.w16(HDA_CORB_CTRL, 0x0003);

        self.w32(HDA_RIRB_BASE, rirb_mem.as_ptr() as u32);
        self.w32(HDA_RIRB_BASE + 4, 0);
        self.w16(HDA_RIRB_SIZE, 0x00 << 4);
        self.w16(HDA_RIRB_CTRL, 0x0002);
        self.w16(HDA_RIRB_CTRL, 0x0003);
        self.w16(HDA_RIRB_INTR, 0x0001);

        let wake_sts = self.r16(HDA_STATESTS);
        if wake_sts == 0 {
            self.w16(HDA_WAKEEN, 0x7FFF);
            for _ in 0..1000 {
                if self.r16(HDA_STATESTS) != 0 {
                    break;
                }
            }
        }

        let codec_mask = self.r16(HDA_STATESTS);
        self.w16(HDA_STATESTS, codec_mask);

        for cad in 0..15 {
            if codec_mask & (1 << cad) != 0 {
                let vendor_id = self.send_verb(cad as u8, 0x00, 0x0F00, 0x00)?;
                let revision = self.send_verb(cad as u8, 0x00, 0x0F04, 0x00)?;
                let node_count = self.send_verb(cad as u8, 0x00, 0x0F02, 0x00)?;

                self.codecs[cad as usize] = Some(HdaCodec {
                    cad: cad as u8,
                    vendor_id,
                    revision,
                    nodes: (node_count >> 20) as u8,
                });
            }
        }

        self.initialized = true;
        Ok(())
    }

    pub fn send_verb(&mut self, cad: u8, node: u8, verb: u32, payload: u32) -> HdaResult<u32> {
        if self.corb_ptr.is_null() {
            return Err(HdaError::InitFailed);
        }

        let corb_entry = verb | (node as u32) << 20 | (cad as u32) << 28 | payload;

        let wp = self.r16(HDA_CORB_WP) as usize;
        let next_wp = (wp + 1) % CORB_ENTRIES;

        unsafe {
            write_volatile(self.corb_ptr.add(wp), corb_entry);
        }
        self.w16(HDA_CORB_WP, next_wp as u16);

        for _ in 0..10000 {
            let rp = self.r16(HDA_RIRB_WP) as usize;
            if rp != self.rirb_rp as usize {
                let response = unsafe { read_volatile(self.rirb_ptr.add(rp * 2)) };
                self.rirb_rp = rp as u8;
                self.w16(HDA_RIRB_WP, 0x4000 | rp as u16);
                return Ok(response);
            }
        }

        Err(HdaError::Timeout)
    }

    pub fn codec(&self, cad: u8) -> Option<&HdaCodec> {
        self.codecs.get(cad as usize).and_then(|c| c.as_ref())
    }

    pub fn codec_count(&self) -> usize {
        self.codecs.iter().filter(|c| c.is_some()).count()
    }

    pub fn allocate_stream(&mut self, _direction: StreamDir) -> Option<u8> {
        for i in 0..32 {
            let s_base = self.mmio_base + SD_OFFSET + i * SD_STRIDE;
            let ctl = self.r32(s_base + SDS_CTRL);
            if ctl & 0x01 == 0 {
                return Some(i as u8);
            }
        }
        None
    }

    pub fn setup_stream_bdl(&self, stream_id: u8, bdl: &[HdaBdlEntry], bdl_count: u32) -> HdaResult {
        let s_base = self.mmio_base + SD_OFFSET + (stream_id as usize) * SD_STRIDE;
        let bdl_addr = bdl.as_ptr() as u64;

        self.w32(s_base + SDS_BDPL, bdl_addr as u32);
        self.w32(s_base + SDS_BDPU, (bdl_addr >> 32) as u32);
        self.w16(s_base + SDS_LVI, if bdl_count > 0 { (bdl_count - 1) as u16 } else { 0 });

        let cbl = bdl.iter().map(|e| e.length).sum();
        self.w32(s_base + SDS_CBL, cbl);

        Ok(())
    }

    pub fn start_stream(&self, stream_id: u8) -> HdaResult {
        let s_base = self.mmio_base + SD_OFFSET + (stream_id as usize) * SD_STRIDE;
        let ctl = self.r32(s_base + SDS_CTRL);
        self.w32(s_base + SDS_CTRL, ctl | 0x0001);
        Ok(())
    }

    pub fn stop_stream(&self, stream_id: u8) -> HdaResult {
        let s_base = self.mmio_base + SD_OFFSET + (stream_id as usize) * SD_STRIDE;
        let ctl = self.r32(s_base + SDS_CTRL);
        self.w32(s_base + SDS_CTRL, ctl & !0x0001);
        Ok(())
    }

    pub fn reset(&self) -> HdaResult {
        let gctl = self.r32(HDA_GCTL);
        self.w32(HDA_GCTL, gctl & !0x0001);

        for _ in 0..1000 {
            if self.r8(HDA_GSTS) & 0x01 == 0 {
                break;
            }
        }

        self.w32(HDA_GCTL, gctl | 0x0001);
        for _ in 0..1000 {
            if self.r8(HDA_GSTS) & 0x01 != 0 {
                return Ok(());
            }
        }

        Err(HdaError::Timeout)
    }
}

pub fn probe(mmio_base: usize) -> HdaResult<HdaController> {
    let ctrl = HdaController::new(mmio_base);

    let ver = ctrl.r16(0x0000);
    if ver == 0xFFFF || ver == 0 {
        return Err(HdaError::NotFound);
    }

    Ok(ctrl)
}
