#![no_std]

use hal::mmio::IoPort;

pub type KbdResult<T = ()> = Result<T, KbdError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbdError {
    ControllerNotFound,
    SelfTestFailed,
    InterfaceTestFailed,
    Disabled,
}

const DATA_PORT: IoPort = IoPort::new(0x60);
const STATUS_PORT: IoPort = IoPort::new(0x64);
const CMD_PORT: IoPort = IoPort::new(0x64);

const STATUS_OUTPUT_FULL: u8 = 0x01;
const STATUS_INPUT_FULL: u8 = 0x02;
const STATUS_SYSTEM_FLAG: u8 = 0x04;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_KBD: u8 = 0xAD;
const CMD_ENABLE_KBD: u8 = 0xAE;
const CMD_SELF_TEST: u8 = 0xAA;
const CMD_KBD_TEST: u8 = 0xAB;

const KBD_ENABLE: u8 = 0xF4;
const KBD_RESET: u8 = 0xFF;
const KBD_SET_LEDS: u8 = 0xED;
const KBD_SET_TYPEMATIC: u8 = 0xF3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    K0, K1, K2, K3, K4, K5, K6, K7, K8, K9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Escape, Tab, CapsLock, LeftShift, RightShift,
    LeftCtrl, RightCtrl, LeftAlt, RightAlt, Space,
    Enter, Backspace, Delete, Insert, Home, End,
    PageUp, PageDown, Up, Down, Left, Right,
    Minus, Equals, LBracket, RBracket, Semicolon,
    Quote, Comma, Period, Slash, Backslash, Tilde,
    Grave, Menu, PrintScreen, ScrollLock, Pause,
    Keypad0, Keypad1, Keypad2, Keypad3, Keypad4,
    Keypad5, Keypad6, Keypad7, Keypad8, Keypad9,
    KeypadDivide, KeypadMultiply, KeypadMinus,
    KeypadPlus, KeypadEnter, KeypadDecimal,
    NumLock,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub pressed: bool,
}

pub struct KeyboardController {
    enabled: bool,
}

impl KeyboardController {
    pub fn new() -> Self {
        Self { enabled: false }
    }

    fn wait_read(&self) {
        for _ in 0..10000 {
            let st = STATUS_PORT.inb();
            if st & STATUS_OUTPUT_FULL != 0 {
                return;
            }
        }
    }

    fn wait_write(&self) {
        for _ in 0..10000 {
            let st = STATUS_PORT.inb();
            if st & STATUS_INPUT_FULL == 0 {
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

    fn read_status(&self) -> u8 {
        STATUS_PORT.inb()
    }

    pub fn init(&mut self) -> KbdResult {
        self.wait_write();
        self.write_cmd(CMD_DISABLE_KBD);
        self.wait_write();
        self.write_cmd(CMD_READ_CONFIG);
        self.wait_read();
        let mut config = self.read_data();

        config &= !0x40;
        config |= 0x04;
        config |= 0x01;

        self.wait_write();
        self.write_cmd(CMD_WRITE_CONFIG);
        self.wait_write();
        self.write_data(config);

        self.wait_write();
        self.write_cmd(CMD_SELF_TEST);
        self.wait_read();
        if self.read_data() != 0x55 {
            return Err(KbdError::SelfTestFailed);
        }

        self.wait_write();
        self.write_cmd(CMD_KBD_TEST);
        self.wait_read();
        if self.read_data() != 0x00 {
            return Err(KbdError::InterfaceTestFailed);
        }

        self.wait_write();
        self.write_cmd(CMD_ENABLE_KBD);

        self.wait_write();
        self.write_data(KBD_RESET);
        self.wait_read();
        if self.read_data() != 0xFA {
            return Err(KbdError::ControllerNotFound);
        }
        self.wait_read();
        let _bat = self.read_data();

        self.enabled = true;
        Ok(())
    }

    pub fn read_event(&self) -> Option<KeyEvent> {
        if self.read_status() & STATUS_OUTPUT_FULL == 0 {
            return None;
        }

        let scancode = self.read_data();
        Some(Self::decode_scancode(scancode))
    }

    pub fn flush(&self) {
        while self.read_status() & STATUS_OUTPUT_FULL != 0 {
            DATA_PORT.inb();
        }
    }

    pub fn set_leds(&self, num_lock: bool, caps_lock: bool, scroll_lock: bool) {
        let mut led = 0u8;
        if scroll_lock { led |= 1; }
        if num_lock { led |= 2; }
        if caps_lock { led |= 4; }

        self.wait_write();
        self.write_data(KBD_SET_LEDS);
        self.wait_read();
        let _ack = self.read_data();
        self.wait_write();
        self.write_data(led);
    }

    fn decode_scancode(scancode: u8) -> KeyEvent {
        let pressed = scancode & 0x80 == 0;
        let code = match scancode & 0x7F {
            0x01 => KeyCode::Escape,
            0x02 => KeyCode::K1, 0x03 => KeyCode::K2,
            0x04 => KeyCode::K3, 0x05 => KeyCode::K4,
            0x06 => KeyCode::K5, 0x07 => KeyCode::K6,
            0x08 => KeyCode::K7, 0x09 => KeyCode::K8,
            0x0A => KeyCode::K9, 0x0B => KeyCode::K0,
            0x0C => KeyCode::Minus, 0x0D => KeyCode::Equals,
            0x0E => KeyCode::Backspace, 0x0F => KeyCode::Tab,
            0x10 => KeyCode::Q, 0x11 => KeyCode::W,
            0x12 => KeyCode::E, 0x13 => KeyCode::R,
            0x14 => KeyCode::T, 0x15 => KeyCode::Y,
            0x16 => KeyCode::U, 0x17 => KeyCode::I,
            0x18 => KeyCode::O, 0x19 => KeyCode::P,
            0x1A => KeyCode::LBracket, 0x1B => KeyCode::RBracket,
            0x1C => KeyCode::Enter, 0x1D => KeyCode::LeftCtrl,
            0x1E => KeyCode::A, 0x1F => KeyCode::S,
            0x20 => KeyCode::D, 0x21 => KeyCode::F,
            0x22 => KeyCode::G, 0x23 => KeyCode::H,
            0x24 => KeyCode::J, 0x25 => KeyCode::K,
            0x26 => KeyCode::L, 0x27 => KeyCode::Semicolon,
            0x28 => KeyCode::Quote, 0x29 => KeyCode::Tilde,
            0x2A => KeyCode::LeftShift, 0x2B => KeyCode::Backslash,
            0x2C => KeyCode::Z, 0x2D => KeyCode::X,
            0x2E => KeyCode::C, 0x2F => KeyCode::V,
            0x30 => KeyCode::B, 0x31 => KeyCode::N,
            0x32 => KeyCode::M, 0x33 => KeyCode::Comma,
            0x34 => KeyCode::Period, 0x35 => KeyCode::Slash,
            0x36 => KeyCode::RightShift, 0x37 => KeyCode::KeypadMultiply,
            0x38 => KeyCode::LeftAlt, 0x39 => KeyCode::Space,
            0x3A => KeyCode::CapsLock,
            0x3B => KeyCode::F1, 0x3C => KeyCode::F2,
            0x3D => KeyCode::F3, 0x3E => KeyCode::F4,
            0x3F => KeyCode::F5, 0x40 => KeyCode::F6,
            0x41 => KeyCode::F7, 0x42 => KeyCode::F8,
            0x43 => KeyCode::F9, 0x44 => KeyCode::F10,
            0x45 => KeyCode::NumLock, 0x46 => KeyCode::ScrollLock,
            0x47 => KeyCode::Keypad7, 0x48 => KeyCode::Keypad8,
            0x49 => KeyCode::Keypad9, 0x4A => KeyCode::KeypadMinus,
            0x4B => KeyCode::Keypad4, 0x4C => KeyCode::Keypad5,
            0x4D => KeyCode::Keypad6, 0x4E => KeyCode::KeypadPlus,
            0x4F => KeyCode::Keypad1, 0x50 => KeyCode::Keypad2,
            0x51 => KeyCode::Keypad3, 0x52 => KeyCode::Keypad0,
            0x53 => KeyCode::KeypadDecimal,
            0x57 => KeyCode::F11, 0x58 => KeyCode::F12,
            _ => KeyCode::Unknown(scancode & 0x7F),
        };

        KeyEvent { code, pressed }
    }
}
