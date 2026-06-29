#![no_std]

use hal::mmio::IoPort;

pub type MouseResult<T = ()> = Result<T, MouseError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseError {
    ControllerNotFound,
    SelfTestFailed,
    Disabled,
    InvalidPacket,
}

const DATA_PORT: IoPort = IoPort::new(0x60);
const CMD_PORT: IoPort = IoPort::new(0x64);

const CMD_DISABLE_KBD: u8 = 0xAD;
const CMD_ENABLE_KBD: u8 = 0xAE;
const CMD_READ_CMDBYTE: u8 = 0x20;
const CMD_WRITE_CMDBYTE: u8 = 0x60;
const CMD_MOUSE_CMD: u8 = 0xD4;

const MOUSE_RESET: u8 = 0xFF;
const MOUSE_ENABLE: u8 = 0xF4;
const MOUSE_DISABLE: u8 = 0xF5;
const MOUSE_SET_DEFAULTS: u8 = 0xF6;
const MOUSE_SET_RATE: u8 = 0xF3;
const MOUSE_SET_RESOLUTION: u8 = 0xE8;
const MOUSE_STATUS_REQUEST: u8 = 0xE9;
const MOUSE_SET_SCALING1: u8 = 0xE6;
const MOUSE_SET_SCALING2: u8 = 0xE7;
const MOUSE_SET_SAMPLE_RATE: u8 = 0xF3;

const MOUSE_ACK: u8 = 0xFA;
const MOUSE_BAT_SUCCESS: u8 = 0xAA;

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy)]
pub struct MousePacket {
    pub buttons: u8,
    pub x: i16,
    pub y: i16,
    pub z: i8,
}

impl MousePacket {
    pub fn is_button_pressed(&self, btn: MouseButton) -> bool {
        match btn {
            MouseButton::Left => self.buttons & 0x01 != 0,
            MouseButton::Right => self.buttons & 0x02 != 0,
            MouseButton::Middle => self.buttons & 0x04 != 0,
        }
    }
}

pub struct MouseController {
    rate: u8,
    resolution: u8,
    sample_rate: u8,
    packet_buf: [u8; 4],
    packet_idx: usize,
    has_wheel: bool,
    enabled: bool,
}

impl MouseController {
    pub fn new() -> Self {
        Self {
            rate: 100,
            resolution: 4,
            sample_rate: 100,
            packet_buf: [0u8; 4],
            packet_idx: 0,
            has_wheel: false,
            enabled: false,
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

    fn mouse_command(&self, cmd: u8) -> u8 {
        self.wait_write();
        self.write_cmd(CMD_MOUSE_CMD);
        self.wait_write();
        self.write_data(cmd);
        self.wait_read();
        self.read_data()
    }

    fn mouse_command_with_data(&self, cmd: u8, data: u8) -> u8 {
        let ack = self.mouse_command(cmd);
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

    fn set_sample_rate(&self, rate: u8) -> bool {
        self.mouse_command_with_data(MOUSE_SET_SAMPLE_RATE, rate) == MOUSE_ACK
    }

    fn read_cmd_byte(&self) -> u8 {
        self.wait_write();
        self.write_cmd(CMD_READ_CMDBYTE);
        self.wait_read();
        self.read_data()
    }

    fn write_cmd_byte(&self, val: u8) {
        self.wait_write();
        self.write_cmd(CMD_WRITE_CMDBYTE);
        self.wait_write();
        self.write_data(val);
    }

    pub fn init(&mut self) -> MouseResult {
        self.wait_write();
        self.write_cmd(CMD_DISABLE_KBD);

        let cmd_byte = self.read_cmd_byte();
        let cmd_byte = cmd_byte | 0x02;
        self.write_cmd_byte(cmd_byte);

        let ack = self.mouse_command(MOUSE_RESET);
        if ack != MOUSE_ACK {
            return Err(MouseError::ControllerNotFound);
        }

        self.wait_read();
        let bat = self.read_data();
        if bat != MOUSE_BAT_SUCCESS {
            self.wait_read();
        }

        let _dev_id = self.read_data();

        self.detect_wheel();

        let ack = self.mouse_command(MOUSE_ENABLE);
        if ack != MOUSE_ACK {
            return Err(MouseError::Disabled);
        }

        self.mouse_command_with_data(MOUSE_SET_SAMPLE_RATE, self.sample_rate);
        self.mouse_command_with_data(MOUSE_SET_RESOLUTION, self.resolution);

        self.wait_write();
        self.write_cmd(CMD_ENABLE_KBD);

        self.enabled = true;
        Ok(())
    }

    fn detect_wheel(&mut self) {
        self.set_sample_rate(200);
        self.set_sample_rate(100);
        self.set_sample_rate(80);

        self.mouse_command(MOUSE_STATUS_REQUEST);
        self.wait_read();
        let _status = self.read_data();
        self.wait_read();
        let _resolution = self.read_data();
        self.wait_read();
        let dev_id = self.read_data();

        self.has_wheel = dev_id == 0x03;
    }

    pub fn read_packet(&mut self) -> Option<MousePacket> {
        let st = CMD_PORT.inb();
        if st & 0x01 == 0 {
            return None;
        }

        let data = self.read_data();

        if self.packet_idx == 0 {
            if data & 0x08 == 0 {
                return None;
            }
        }

        self.packet_buf[self.packet_idx] = data;
        self.packet_idx += 1;

        let packet_size = if self.has_wheel { 4 } else { 3 };

        if self.packet_idx < packet_size {
            return None;
        }

        self.packet_idx = 0;

        let b0 = self.packet_buf[0];
        let b1 = self.packet_buf[1];
        let b2 = self.packet_buf[2];

        let x = if b0 & 0x10 != 0 {
            (b1 as i16) - 256
        } else {
            b1 as i16
        };

        let y = if b0 & 0x20 != 0 {
            -((b2 as i16) - 256)
        } else {
            -(b2 as i16)
        };

        let z = if self.has_wheel {
            (self.packet_buf[3] as i8).wrapping_mul(-1)
        } else {
            0
        };

        Some(MousePacket {
            buttons: b0 & 0x07,
            x,
            y,
            z,
        })
    }
}
