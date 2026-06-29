#![no_std]

use hal::mmio::IoPort;

pub type TouchpadResult<T = ()> = Result<T, TouchpadError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadError {
    NotFound,
    InitFailed,
    InvalidPacket,
    I2cError,
}

const DATA_PORT: IoPort = IoPort::new(0x60);
const CMD_PORT: IoPort = IoPort::new(0x64);
const CMD_MOUSE_CMD: u8 = 0xD4;

const MOUSE_ACK: u8 = 0xFA;
const MOUSE_RESET: u8 = 0xFF;
const MOUSE_ENABLE: u8 = 0xF4;
const MOUSE_SET_RESOLUTION: u8 = 0xE8;
const MOUSE_SET_SAMPLE_RATE: u8 = 0xF3;
const MOUSE_STATUS_REQUEST: u8 = 0xE9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadVendor {
    Synaptics,
    Alps,
    Elan,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct TouchpadState {
    pub x: u16,
    pub y: u16,
    pub z: u8,
    pub finger: bool,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FingerPosition {
    pub x: u16,
    pub y: u16,
    pub z: u8,
    pub contact: bool,
}

pub struct Touchpad {
    vendor: TouchpadVendor,
    absolute_mode: bool,
    width_mm: u16,
    height_mm: u16,
    x_max: u16,
    y_max: u16,
    resolution: u8,
}

impl Touchpad {
    pub fn new() -> Self {
        Self {
            vendor: TouchpadVendor::Unknown,
            absolute_mode: false,
            width_mm: 100,
            height_mm: 60,
            x_max: 4096,
            y_max: 2048,
            resolution: 4,
        }
    }

    fn wait_write(&self) {
        for _ in 0..10000 {
            let st = CMD_PORT.inb();
            if st & 0x02 == 0 {
                return;
            }
        }
    }

    fn wait_read(&self) {
        for _ in 0..10000 {
            let st = CMD_PORT.inb();
            if st & 0x01 != 0 {
                return;
            }
        }
    }

    fn read_data(&self) -> u8 {
        DATA_PORT.inb()
    }

    fn write_data(&self, val: u8) {
        DATA_PORT.outb(val)
    }

    fn write_cmd(&self, cmd: u8) {
        CMD_PORT.outb(cmd)
    }

    fn ps2_command(&self, cmd: u8) -> u8 {
        self.wait_write();
        self.write_cmd(CMD_MOUSE_CMD);
        self.wait_write();
        self.write_data(cmd);
        self.wait_read();
        self.read_data()
    }

    fn ps2_command_data(&self, cmd: u8, data: u8) -> u8 {
        let ack = self.ps2_command(cmd);
        if ack != MOUSE_ACK {
            return ack;
        }
        self.wait_write();
        self.write_cmd(CMD_MOUSE_CMD);
        self.wait_write();
        self.write_data(data);
        self.wait_read();
        self.read_data()
    }

    fn set_resolution(&self, res: u8) -> bool {
        self.ps2_command_data(MOUSE_SET_RESOLUTION, res) == MOUSE_ACK
    }

    fn set_sample_rate(&self, rate: u8) -> bool {
        self.ps2_command_data(MOUSE_SET_SAMPLE_RATE, rate) == MOUSE_ACK
    }

    pub fn probe_synaptics(&mut self) -> TouchpadResult {
        self.set_sample_rate(200);
        self.set_sample_rate(100);
        self.set_sample_rate(80);

        let ack = self.ps2_command(MOUSE_STATUS_REQUEST);
        if ack != MOUSE_ACK {
            return Err(TouchpadError::NotFound);
        }

        self.wait_read();
        let _status = self.read_data();
        self.wait_read();
        let _res = self.read_data();
        self.wait_read();
        let dev_id = self.read_data();

        if dev_id == 0x47 {
            self.vendor = TouchpadVendor::Synaptics;
            self.absolute_mode = true;
            self.enable_absolute_mode();
            Ok(())
        } else {
            Err(TouchpadError::NotFound)
        }
    }

    fn enable_absolute_mode(&self) {
        self.set_sample_rate(200);
        self.set_sample_rate(100);
        self.set_sample_rate(80);

        self.ps2_command_data(0xE8, 0x01);
        self.ps2_command_data(0xE8, 0x02);
        self.ps2_command_data(0xE8, 0x04);
        self.ps2_command_data(0xE8, 0x08);

        self.ps2_command_data(0xE8, 0x01);
        self.ps2_command_data(0xE8, 0x01);
    }

    pub fn init(&mut self) -> TouchpadResult {
        let ack = self.ps2_command(MOUSE_RESET);
        if ack != MOUSE_ACK {
            return Err(TouchpadError::NotFound);
        }
        self.wait_read();
        let _bat = self.read_data();
        self.wait_read();
        let _dev_id = self.read_data();

        if self.probe_synaptics().is_ok() {
            return Ok(());
        }

        Err(TouchpadError::InitFailed)
    }

    pub fn read_state(&mut self) -> Option<TouchpadState> {
        let st = CMD_PORT.inb();
        if st & 0x01 == 0 {
            return None;
        }

        let b0 = self.read_data();
        if b0 & 0x08 == 0 {
            return None;
        }

        self.wait_read();
        let b1 = self.read_data();
        self.wait_read();
        let b2 = self.read_data();

        if self.absolute_mode && self.vendor == TouchpadVendor::Synaptics {
            let self_addr = self as *const Self;
            let x = ((b1 & 0x1F) as u16) << 8 | b2 as u16;
            self.wait_read();
            let b3 = self.read_data();
            self.wait_read();
            let b4 = self.read_data();
            self.wait_read();
            let b5 = self.read_data();

            let y = ((b4 & 0x1F) as u16) << 8 | b5 as u16;
            let z = b3;
            let finger = b0 & 0x20 != 0;

            Some(TouchpadState {
                x,
                y,
                z,
                finger,
                left_button: b0 & 0x01 != 0,
                right_button: b0 & 0x02 != 0,
                middle_button: false,
            })
        } else {
            let x = if b0 & 0x10 != 0 { (b1 as i16) - 256 } else { b1 as i16 };
            let y = if b0 & 0x20 != 0 { -((b2 as i16) - 256) } else { -(b2 as i16) };

            Some(TouchpadState {
                x: x as u16,
                y: y as u16,
                z: 0,
                finger: true,
                left_button: b0 & 0x01 != 0,
                right_button: b0 & 0x02 != 0,
                middle_button: b0 & 0x04 != 0,
            })
        }
    }
}
