#![no_std]

use hal::mmio::Mmio;

pub type Ac97Result<T = ()> = Result<T, Ac97Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ac97Error {
    NotFound,
    CodecNotReady,
    ResetFailed,
    InvalidRate,
    DmaError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    Rate8000 = 8000,
    Rate11025 = 11025,
    Rate16000 = 16000,
    Rate22050 = 22050,
    Rate32000 = 32000,
    Rate44100 = 44100,
    Rate48000 = 48000,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    S16Le,
    S20Le,
    S24Le,
}

pub const AC97_RESET: u8 = 0x00;
pub const AC97_MASTER_VOL: u8 = 0x02;
pub const AC97_PCM_OUT_VOL: u8 = 0x18;
pub const AC97_PCM_IN_VOL: u8 = 0x1A;
pub const AC97_EXT_AUDIO_ID: u8 = 0x28;
pub const AC97_EXT_AUDIO_CTRL: u8 = 0x2A;
pub const AC97_PCM_FRONT_DAC_RATE: u8 = 0x2C;
pub const AC97_PCM_LR_ADC_RATE: u8 = 0x32;
pub const AC97_VENDOR_ID1: u8 = 0x7C;
pub const AC97_VENDOR_ID2: u8 = 0x7E;

pub const AC97_POWER_DOWN: u8 = 0x26;

pub const AC97_INDEX: u16 = 0x3F0;
pub const AC97_DATA: u16 = 0x3F2;

pub struct Ac97Controller {
    mmio: Mmio,
    codecs: [Option<Ac97Codec>; 4],
    initialized: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Ac97Codec {
    pub id: u8,
    pub vendor_id: u32,
    pub extended_audio: u16,
}

impl Ac97Codec {
    pub fn supports_audiorate(&self, _rate: SampleRate) -> bool {
        self.extended_audio & 0x0001 != 0
    }
}

impl Ac97Controller {
    pub fn new(mmio: Mmio) -> Self {
        Self { mmio, codecs: [None, None, None, None], initialized: false }
    }

    fn codec_ready(&self) -> bool {
        let status = self.mmio.r16(AC97_POWER_DOWN as usize);
        status != 0xFFFF
    }

    pub fn probe(&mut self) -> Ac97Result<usize> {
        let mut found = 0;

        for codec_id in 0..4 {
            let _primary = if codec_id == 0 { 0x8000u16 } else { 0x0000 };
            let id1 = self.mmio.r16(AC97_VENDOR_ID1 as usize);
            let id2 = self.mmio.r16(AC97_VENDOR_ID2 as usize);

            if id1 != 0xFFFF && id1 != 0 {
                let vendor_id = ((id1 as u32) << 16) | id2 as u32;
                let ext_audio = self.mmio.r16(AC97_EXT_AUDIO_ID as usize);

                self.codecs[codec_id] = Some(Ac97Codec {
                    id: codec_id as u8,
                    vendor_id,
                    extended_audio: ext_audio,
                });
                found += 1;
            }
        }

        self.initialized = found > 0;
        if found == 0 {
            Err(Ac97Error::NotFound)
        } else {
            Ok(found)
        }
    }

    pub fn codec(&self, index: usize) -> Option<&Ac97Codec> {
        self.codecs.get(index).and_then(|c| c.as_ref())
    }

    pub fn reset(&self) -> Ac97Result {
        self.mmio.w16(AC97_RESET as usize, 0x0000);
        for _ in 0..1000 {
            if self.codec_ready() {
                return Ok(());
            }
        }
        Err(Ac97Error::ResetFailed)
    }

    pub fn cold_reset(&self) -> Ac97Result {
        self.mmio.w16(AC97_POWER_DOWN as usize, 0x000F);
        for _ in 0..1000 {
            if self.codec_ready() {
                return Ok(());
            }
        }
        Err(Ac97Error::ResetFailed)
    }

    pub fn set_pcm_out_rate(&self, rate: SampleRate) -> Ac97Result {
        if !self.codec_ready() {
            return Err(Ac97Error::CodecNotReady);
        }
        self.mmio.w16(AC97_PCM_FRONT_DAC_RATE as usize, rate as u16);
        let readback = self.mmio.r16(AC97_PCM_FRONT_DAC_RATE as usize);
        if readback != rate as u16 {
            return Err(Ac97Error::InvalidRate);
        }
        Ok(())
    }

    pub fn set_pcm_in_rate(&self, rate: SampleRate) -> Ac97Result {
        if !self.codec_ready() {
            return Err(Ac97Error::CodecNotReady);
        }
        self.mmio.w16(AC97_PCM_LR_ADC_RATE as usize, rate as u16);
        let readback = self.mmio.r16(AC97_PCM_LR_ADC_RATE as usize);
        if readback != rate as u16 {
            return Err(Ac97Error::InvalidRate);
        }
        Ok(())
    }

    pub fn set_master_volume(&self, left: u8, right: u8) {
        let val = ((right as u16) & 0x3F) | (((left as u16) & 0x3F) << 8);
        self.mmio.w16(AC97_MASTER_VOL as usize, val);
    }

    pub fn set_pcm_out_volume(&self, left: u8, right: u8) {
        let val = ((right as u16) & 0x3F) | (((left as u16) & 0x3F) << 8);
        self.mmio.w16(AC97_PCM_OUT_VOL as usize, val);
    }

    pub fn set_pcm_in_volume(&self, left: u8, right: u8) {
        let val = ((right as u16) & 0x3F) | (((left as u16) & 0x3F) << 8);
        self.mmio.w16(AC97_PCM_IN_VOL as usize, val);
    }

    pub fn master_volume(&self) -> (u8, u8) {
        let val = self.mmio.r16(AC97_MASTER_VOL as usize);
        ( (val >> 8) as u8 & 0x3F, val as u8 & 0x3F )
    }

    pub fn vendor_id(&self) -> Option<u32> {
        self.codec(0).map(|c| c.vendor_id)
    }
}

pub struct Ac97PcmOut {
    dma_buffer: Option<&'static mut [u16]>,
    buffer_size: usize,
}

impl Ac97PcmOut {
    pub fn new() -> Self {
        Self { dma_buffer: None, buffer_size: 0 }
    }

    pub fn prepare(&mut self, buffer: &'static mut [u16]) {
        self.buffer_size = buffer.len();
        self.dma_buffer = Some(buffer);
    }

    pub fn write_sample(&mut self, left: i16, right: i16, position: usize) {
        if let Some(ref mut buf) = self.dma_buffer {
            if position * 2 + 1 < buf.len() {
                buf[position * 2] = left as u16;
                buf[position * 2 + 1] = right as u16;
            }
        }
    }
}
