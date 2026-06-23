// src/keyboard.rs

const KEYBOARD_PORT: u16 = 0x60;

/// Ждёт нажатия Enter
pub fn wait_for_enter() {
    loop {
        let scancode = unsafe { read_port(KEYBOARD_PORT) };
        if scancode == 0x1C {
            break;
        }
    }
}

/// Выключение через QEMU
pub fn shutdown() -> ! {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x604u16,
            in("al") 0x00u8,
        );
    }
    loop {}
}

unsafe fn read_port(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") value, in("dx") port);
    }
    value
}
