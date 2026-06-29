#![no_std]

use hal::mmio::Mmio;

pub type HubResult<T = ()> = Result<T, HubError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubError {
    NotFound,
    InitFailed,
    PortError,
    PowerFault,
    OverCurrent,
    DeviceEnumFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    Disconnected,
    Connected,
    Enabled,
    Suspended,
    Resetting,
    PowerFault,
}

#[derive(Debug, Clone, Copy)]
pub struct PortState {
    pub port: u8,
    pub status: PortStatus,
    pub speed: PortSpeed,
    pub power_on: bool,
    pub overcurrent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSpeed {
    None,
    Low,
    Full,
    High,
    Super,
}

#[derive(Debug, Clone, Copy)]
pub struct HubDescriptor {
    pub b_desc_length: u8,
    pub b_desc_type: u8,
    pub b_nbr_ports: u8,
    pub w_hub_characteristics: u16,
    pub b_pwr_on_2_pwr_good: u8,
    pub b_hub_contr_current: u8,
}

pub struct UsbHub {
    mmio: Mmio,
    ports: u8,
    port_states: [PortState; 16],
    power_good_delay: u8,
    initialized: bool,
}

impl UsbHub {
    pub fn new(base: usize, size: usize) -> Self {
        let mmio = unsafe { Mmio::new(base, size) };
        Self {
            mmio,
            ports: 0,
            port_states: [PortState {
                port: 0,
                status: PortStatus::Disconnected,
                speed: PortSpeed::None,
                power_on: false,
                overcurrent: false,
            }; 16],
            power_good_delay: 100,
            initialized: false,
        }
    }

    pub fn init(&mut self) -> HubResult {
        let ver = self.mmio.r16(0x00);
        if ver == 0xFFFF || ver == 0 {
            return Err(HubError::NotFound);
        }

        self.mmio.w32(0x04, 0x01);
        for _ in 0..1000 {
            if self.mmio.r32(0x04) & 0x01 != 0 {
                break;
            }
        }

        let desc_data = self.read_hub_descriptor();
        self.ports = desc_data.b_nbr_ports.min(16);
        self.power_good_delay = desc_data.b_pwr_on_2_pwr_good;

        for port in 0..self.ports {
            self.power_port(port);
            self.port_states[port as usize] = PortState {
                port,
                status: PortStatus::Disconnected,
                speed: PortSpeed::None,
                power_on: true,
                overcurrent: false,
            };
        }

        self.initialized = true;
        Ok(())
    }

    fn read_hub_descriptor(&self) -> HubDescriptor {
        HubDescriptor {
            b_desc_length: self.mmio.r8(0x10),
            b_desc_type: self.mmio.r8(0x11),
            b_nbr_ports: self.mmio.r8(0x12),
            w_hub_characteristics: self.mmio.r16(0x13),
            b_pwr_on_2_pwr_good: self.mmio.r8(0x15),
            b_hub_contr_current: self.mmio.r8(0x16),
        }
    }

    pub fn port_count(&self) -> u8 {
        self.ports
    }

    fn port_reg(&self, port: u8) -> usize {
        0x100 + (port as usize) * 0x10
    }

    pub fn port_status(&self, port: u8) -> u32 {
        if port >= self.ports {
            return 0;
        }
        self.mmio.r32(self.port_reg(port))
    }

    pub fn port_change(&self, port: u8) -> u16 {
        if port >= self.ports {
            return 0;
        }
        self.mmio.r16(self.port_reg(port) + 4)
    }

    pub fn power_port(&self, port: u8) {
        let reg = self.port_reg(port);
        self.mmio.w32(reg, self.mmio.r32(reg) | 0x0000_0004);
    }

    pub fn reset_port(&self, port: u8) -> HubResult {
        let reg = self.port_reg(port);
        self.mmio.w32(reg, self.mmio.r32(reg) | 0x0000_0010);

        for _ in 0..10000 {
            let sts = self.mmio.r32(reg);
            if sts & 0x0000_0010 == 0 {
                return Ok(());
            }
        }

        self.mmio.w32(reg, self.mmio.r32(reg) & !0x0000_0010);
        Ok(())
    }

    pub fn enable_port(&self, port: u8) {
        let reg = self.port_reg(port);
        self.mmio.w32(reg, self.mmio.r32(reg) | 0x0000_0002);
    }

    pub fn disable_port(&self, port: u8) {
        let reg = self.port_reg(port);
        self.mmio.w32(reg, self.mmio.r32(reg) & !0x0000_0002);
    }

    pub fn port_speed(&self, port: u8) -> PortSpeed {
        let sts = self.port_status(port);
        let speed_code = (sts >> 2) & 0x03;
        match speed_code {
            0 => PortSpeed::Full,
            1 => PortSpeed::Low,
            2 => PortSpeed::High,
            3 => PortSpeed::Super,
            _ => PortSpeed::None,
        }
    }

    pub fn detect_changes(&mut self) -> Option<u8> {
        for port in 0..self.ports {
            let change = self.port_change(port);
            if change & 0x01 != 0 {
                let sts = self.port_status(port);
                let connected = sts & 0x01 != 0;

                self.clear_port_change(port, 0x01);

                self.port_states[port as usize].status = if connected {
                    PortStatus::Connected
                } else {
                    PortStatus::Disconnected
                };
                self.port_states[port as usize].speed = self.port_speed(port);

                return Some(port);
            }

            if change & 0x08 != 0 {
                self.clear_port_change(port, 0x08);
                self.port_states[port as usize].overcurrent = true;
            }
        }

        None
    }

    pub fn clear_port_change(&self, port: u8, change: u16) {
        let reg = self.port_reg(port) + 4;
        self.mmio.w16(reg, change);
    }

    pub fn poll_ports(&self) -> impl Iterator<Item = &PortState> {
        self.port_states.iter().filter(|p| p.status != PortStatus::Disconnected)
    }

    pub fn state(&self, port: u8) -> Option<&PortState> {
        if port < self.ports {
            Some(&self.port_states[port as usize])
        } else {
            None
        }
    }
}
