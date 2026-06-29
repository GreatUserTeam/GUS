#![no_std]

use hal::mmio::IoPort;

pub const VGA_TEXT_BASE: usize = 0xB8000;
pub const VGA_COLS: usize = 80;
pub const VGA_ROWS: usize = 25;

#[allow(dead_code)]
const CRT_INDEX: u16 = 0x3D4;
#[allow(dead_code)]
const CRT_DATA: u16 = 0x3D5;
#[allow(dead_code)]
const MISC_OUT: u16 = 0x3C2;
#[allow(dead_code)]
const SEQUENCER_INDEX: u16 = 0x3C4;
#[allow(dead_code)]
const SEQUENCER_DATA: u16 = 0x3C5;
#[allow(dead_code)]
const GC_INDEX: u16 = 0x3CE;
#[allow(dead_code)]
const GC_DATA: u16 = 0x3CF;
#[allow(dead_code)]
const AC_INDEX: u16 = 0x3C0;
#[allow(dead_code)]
const AC_DATA: u16 = 0x3C1;
#[allow(dead_code)]
const DAC_WRITE: u16 = 0x3C8;
#[allow(dead_code)]
const DAC_DATA: u16 = 0x3C9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VgaMode {
    Text80x25,
    Text80x50,
    Graphics320x200x256,
    Graphics640x480x16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    LightMagenta = 13,
    Yellow = 14,
    White = 15,
}

fn make_attr(fg: Color, bg: Color, blink: bool) -> u8 {
    (bg as u8) << 4 | (fg as u8) | if blink { 0x80 } else { 0 }
}

#[derive(Debug, Clone, Copy)]
pub struct VgaChar {
    pub ch: u8,
    pub fg: Color,
    pub bg: Color,
    pub blink: bool,
}

impl VgaChar {
    pub const fn new(ch: u8, fg: Color, bg: Color) -> Self {
        Self { ch, fg, bg, blink: false }
    }

    pub const fn space() -> Self {
        Self::new(b' ', Color::LightGray, Color::Black)
    }

    fn to_word(self) -> u16 {
        self.ch as u16 | ((make_attr(self.fg, self.bg, self.blink) as u16) << 8)
    }
}

pub struct VgaTextMode {
    buffer: &'static mut [u16],
    cursor_x: usize,
    cursor_y: usize,
    rows: usize,
    cols: usize,
}

impl VgaTextMode {
    pub fn new(base: usize, rows: usize, cols: usize) -> Self {
        let buffer = unsafe { core::slice::from_raw_parts_mut(base as *mut u16, rows * cols) };
        Self { buffer, cursor_x: 0, cursor_y: 0, rows, cols }
    }

    pub fn new_80x25() -> Self {
        Self::new(VGA_TEXT_BASE, VGA_ROWS, VGA_COLS)
    }

    fn index(&self, row: usize, col: usize) -> usize {
        row * self.cols + col
    }

    pub fn write_char(&mut self, row: usize, col: usize, ch: VgaChar) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let idx = self.index(row, col);
        self.buffer[idx] = ch.to_word();
    }

    pub fn read_char(&self, row: usize, col: usize) -> Option<VgaChar> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let idx = self.index(row, col);
        let word = self.buffer[idx];
        let ch = word as u8;
        let attr = (word >> 8) as u8;
        let blink = attr & 0x80 != 0;
        let bg = Color::from_u8((attr >> 4) & 0x07);
        let fg = Color::from_u8(attr & 0x0F);
        Some(VgaChar { ch, fg: fg.unwrap_or(Color::LightGray), bg: bg.unwrap_or(Color::Black), blink })
    }

    pub fn scroll(&mut self) {
        for row in 1..self.rows {
            for col in 0..self.cols {
                let src = self.index(row, col);
                let dst = self.index(row - 1, col);
                self.buffer[dst] = self.buffer[src];
            }
        }
        let last_row = self.rows - 1;
        for col in 0..self.cols {
            self.buffer[self.index(last_row, col)] = VgaChar::space().to_word();
        }
    }

    pub fn clear(&mut self, bg: Color) {
        let space = VgaChar { ch: b' ', fg: Color::LightGray, bg, blink: false };
        let word = space.to_word();
        for i in 0..(self.rows * self.cols) {
            self.buffer[i] = word;
        }
    }

    pub fn set_cursor_pos(&self, row: usize, col: usize) {
        let pos = (row * self.cols + col) as u16;
        let crt_idx = IoPort::new(CRT_INDEX);
        let crt_data = IoPort::new(CRT_DATA);

        crt_idx.outw(0x0E);
        crt_data.outw((pos >> 8) as u16);
        crt_idx.outw(0x0F);
        crt_data.outw(pos);
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor_y, self.cursor_x)
    }

    pub fn print(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                b'\n' => {
                    self.cursor_x = 0;
                    self.cursor_y += 1;
                    if self.cursor_y >= self.rows {
                        self.scroll();
                        self.cursor_y = self.rows - 1;
                    }
                }
                b'\r' => {
                    self.cursor_x = 0;
                }
                b'\t' => {
                    let tab_stops = 8;
                    let next = (self.cursor_x / tab_stops + 1) * tab_stops;
                    self.cursor_x = next.min(self.cols - 1);
                }
                _ => {
                    if self.cursor_x >= self.cols {
                        self.cursor_x = 0;
                        self.cursor_y += 1;
                        if self.cursor_y >= self.rows {
                            self.scroll();
                            self.cursor_y = self.rows - 1;
                        }
                    }
                    let ch = VgaChar {
                        ch: byte,
                        fg: Color::LightGray,
                        bg: Color::Black,
                        blink: false,
                    };
                    self.write_char(self.cursor_y, self.cursor_x, ch);
                    self.cursor_x += 1;
                }
            }
        }
        self.set_cursor_pos(self.cursor_y, self.cursor_x);
    }
}

impl Color {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Black),
            1 => Some(Self::Blue),
            2 => Some(Self::Green),
            3 => Some(Self::Cyan),
            4 => Some(Self::Red),
            5 => Some(Self::Magenta),
            6 => Some(Self::Brown),
            7 => Some(Self::LightGray),
            8 => Some(Self::DarkGray),
            9 => Some(Self::LightBlue),
            10 => Some(Self::LightGreen),
            11 => Some(Self::LightCyan),
            12 => Some(Self::LightRed),
            13 => Some(Self::LightMagenta),
            14 => Some(Self::Yellow),
            15 => Some(Self::White),
            _ => None,
        }
    }
}

pub fn vga_write_reg(index: u16, data: u16) {
    IoPort::new(index).outw(data);
}

pub fn vga_read_reg(index: u16) -> u8 {
    IoPort::new(index).inw() as u8
}

pub fn set_video_mode(mode: VgaMode) {
    match mode {
        VgaMode::Text80x25 => {}
        VgaMode::Text80x50 => {
            let seq_idx = IoPort::new(SEQUENCER_INDEX);
            let seq_data = IoPort::new(SEQUENCER_DATA);
            let crt_idx = IoPort::new(CRT_INDEX);
            let crt_data = IoPort::new(CRT_DATA);

            seq_idx.outw(0x01);
            let sr1 = seq_data.inw();
            seq_idx.outw(0x01);
            seq_data.outw(sr1 & !0x20);
            crt_idx.outw(0x09);
            crt_data.outw(0x80);
            crt_idx.outw(0x12);
            let cr12 = crt_data.inw();
            crt_idx.outw(0x12);
            crt_data.outw(cr12 & !0x80);
            crt_idx.outw(0x06);
            crt_data.outw(0x3F);
        }
        VgaMode::Graphics320x200x256 => {}
        VgaMode::Graphics640x480x16 => {}
    }
}
