#![no_std]



#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub base_addr: usize,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn black() -> Self {
        Self::new(0, 0, 0, 255)
    }

    pub const fn white() -> Self {
        Self::new(255, 255, 255, 255)
    }

    pub const fn red() -> Self {
        Self::new(255, 0, 0, 255)
    }

    pub const fn green() -> Self {
        Self::new(0, 255, 0, 255)
    }

    pub const fn blue() -> Self {
        Self::new(0, 0, 255, 255)
    }
}

pub struct Framebuffer {
    info: FramebufferInfo,
    front: &'static mut [u8],
    back: Option<&'static mut [u8]>,
    double_buffered: bool,
}

impl Framebuffer {
    pub fn new(info: FramebufferInfo) -> Self {
        let size = info.pitch * info.height;
        let front = unsafe { core::slice::from_raw_parts_mut(info.base_addr as *mut u8, size) };
        Self { info, front, back: None, double_buffered: false }
    }

    pub fn info(&self) -> &FramebufferInfo {
        &self.info
    }

    pub fn width(&self) -> usize {
        self.info.width
    }

    pub fn height(&self) -> usize {
        self.info.height
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }

        let bpp = self.info.bpp;
        let offset = y * self.info.pitch + x * (bpp as usize / 8);
        let buf = self.active_buffer_mut();

        match bpp {
            32 => {
                buf[offset] = color.b;
                buf[offset + 1] = color.g;
                buf[offset + 2] = color.r;
                buf[offset + 3] = color.a;
            }
            24 => {
                buf[offset] = color.b;
                buf[offset + 1] = color.g;
                buf[offset + 2] = color.r;
            }
            16 => {
                let rgb565 = ((color.r as u16 >> 3) << 11)
                    | ((color.g as u16 >> 2) << 5)
                    | (color.b as u16 >> 3);
                buf[offset] = rgb565 as u8;
                buf[offset + 1] = (rgb565 >> 8) as u8;
            }
            8 => {
                buf[offset] = color.r;
            }
            _ => {}
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> Option<Color> {
        if x >= self.info.width || y >= self.info.height {
            return None;
        }

        let buf = self.active_buffer();
        let offset = y * self.info.pitch + x * (self.info.bpp as usize / 8);

        match self.info.bpp {
            32 => Some(Color {
                b: buf[offset],
                g: buf[offset + 1],
                r: buf[offset + 2],
                a: buf[offset + 3],
            }),
            24 => Some(Color {
                b: buf[offset],
                g: buf[offset + 1],
                r: buf[offset + 2],
                a: 255,
            }),
            _ => None,
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        let x_end = (x + w).min(self.info.width);
        let y_end = (y + h).min(self.info.height);

        for py in y..y_end {
            for px in x..x_end {
                self.set_pixel(px, py, color);
            }
        }
    }

    pub fn clear(&mut self, color: Color) {
        self.fill_rect(0, 0, self.info.width, self.info.height, color);
    }

    pub fn blit(&mut self, data: &[u8], x: usize, y: usize, w: usize, h: usize) {
        let bpp = self.info.bpp as usize / 8;
        for row in 0..h {
            if y + row >= self.info.height {
                break;
            }
            for col in 0..w {
                if x + col >= self.info.width {
                    break;
                }
                let src_offset = (row * w + col) * bpp;
                let pixel = match bpp {
                    3 => Color {
                        r: data[src_offset],
                        g: data[src_offset + 1],
                        b: data[src_offset + 2],
                        a: 255,
                    },
                    4 => Color {
                        r: data[src_offset],
                        g: data[src_offset + 1],
                        b: data[src_offset + 2],
                        a: data[src_offset + 3],
                    },
                    _ => continue,
                };
                self.set_pixel(x + col, y + row, pixel);
            }
        }
    }

    pub fn enable_double_buffer(&mut self, back_buffer_addr: Option<usize>) {
        let size = self.info.pitch * self.info.height;
        let back = match back_buffer_addr {
            Some(addr) => unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, size) },
            None => return,
        };
        self.back = Some(back);
        self.double_buffered = true;
    }

    pub fn swap_buffers(&mut self) {
        if !self.double_buffered {
            return;
        }

        if let Some(back) = &self.back {
            let size = self.info.pitch * self.info.height;
            let front = &mut self.front[..size];
            front.copy_from_slice(&back[..size]);
        }
    }

    pub fn flush(&self) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn active_buffer(&self) -> &[u8] {
        if self.double_buffered {
            if let Some(back) = &self.back {
                return back;
            }
        }
        self.front
    }

    fn active_buffer_mut(&mut self) -> &mut [u8] {
        if self.double_buffered {
            if let Some(back) = &mut self.back {
                return back;
            }
        }
        self.front
    }

    pub fn draw_char(&mut self, x: usize, y: usize, ch: u8, fg: Color, bg: Color, font: &[u8; 4096]) {
        if x + 8 > self.info.width || y + 16 > self.info.height {
            return;
        }

        let glyph_offset = (ch as usize) * 16;
        for row in 0..16 {
            let byte = font[glyph_offset + row];
            for col in 0..8 {
                if byte & (0x80 >> col) != 0 {
                    self.set_pixel(x + col, y + row, fg);
                } else {
                    self.set_pixel(x + col, y + row, bg);
                }
            }
        }
    }

    pub fn draw_string(&mut self, x: usize, y: usize, s: &str, fg: Color, bg: Color, font: &[u8; 4096]) {
        for (i, c) in s.bytes().enumerate() {
            self.draw_char(x + i * 8, y, c, fg, bg, font);
        }
    }
}
