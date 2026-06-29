#![no_std]

pub type BtResult<T = ()> = Result<T, BtError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtError {
    NotFound,
    HciInitFailed,
    CommandTimeout,
    ConnectionFailed,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtTransport {
    Usb,
    Uart,
    Virtual,
}

#[derive(Debug, Clone, Copy)]
pub struct BdAddr(pub [u8; 6]);

impl BdAddr {
    pub fn from_bytes(b: &[u8; 6]) -> Self {
        Self(*b)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LePhy {
    Le1M,
    Le2M,
    LeCoded,
}

#[derive(Debug, Clone, Copy)]
pub struct BtDeviceInfo {
    pub address: BdAddr,
    pub name: [u8; 248],
    pub rssi: i8,
    pub eir: [u8; 240],
}

#[derive(Debug, Clone, Copy)]
pub enum HciPacketType {
    Command = 0x01,
    AclData = 0x02,
    SyncData = 0x03,
    Event = 0x04,
    IsoData = 0x05,
}

#[derive(Debug, Clone, Copy)]
pub struct HciCommand {
    pub opcode: u16,
    pub params: [u8; 16],
    pub param_len: u8,
}

pub struct BluetoothController {
    transport: BtTransport,
    hci_handle: u16,
    initialized: bool,
}

impl BluetoothController {
    pub fn new(transport: BtTransport) -> Self {
        Self { transport, hci_handle: 0, initialized: false }
    }

    pub fn init(&mut self) -> BtResult {
        self.initialized = true;
        Ok(())
    }

    pub fn send_hci_cmd(&self, cmd: &HciCommand) -> BtResult {
        match self.transport {
            BtTransport::Virtual => Ok(()),
            _ => Err(BtError::HciInitFailed),
        }
    }

    pub fn reset(&self) -> BtResult {
        let cmd = HciCommand {
            opcode: 0x0C03,
            params: [0u8; 16],
            param_len: 0,
        };
        self.send_hci_cmd(&cmd)
    }

    pub fn set_event_mask(&self) -> BtResult {
        let mut cmd = HciCommand {
            opcode: 0x0C01,
            params: [0u8; 16],
            param_len: 8,
        };
        cmd.params[..8].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x3F]);
        self.send_hci_cmd(&cmd)
    }

    pub fn write_local_name(&self, name: &[u8]) -> BtResult {
        let mut cmd = HciCommand {
            opcode: 0x0C13,
            params: [0u8; 16],
            param_len: name.len().min(248) as u8,
        };
        let len = name.len().min(248);
        cmd.params[..len].copy_from_slice(&name[..len]);
        self.send_hci_cmd(&cmd)
    }

    pub fn le_set_advertising_enable(&self, enable: bool) -> BtResult {
        let mut cmd = HciCommand {
            opcode: 0x200A,
            params: [0u8; 16],
            param_len: 1,
        };
        cmd.params[0] = if enable { 1 } else { 0 };
        self.send_hci_cmd(&cmd)
    }

    pub fn le_set_advertising_data(&self, data: &[u8]) -> BtResult {
        let len = data.len().min(31);
        let mut cmd = HciCommand {
            opcode: 0x2008,
            params: [0u8; 16],
            param_len: 1 + len as u8,
        };
        cmd.params[0] = len as u8;
        cmd.params[1..=len].copy_from_slice(&data[..len]);
        self.send_hci_cmd(&cmd)
    }

    pub fn le_scan(&self, enable: bool, passive: bool) -> BtResult {
        let mut cmd = HciCommand {
            opcode: 0x200C,
            params: [0u8; 16],
            param_len: 2,
        };
        cmd.params[0] = if enable { 1 } else { 0 };
        cmd.params[1] = if passive { 0x00 } else { 0x01 };
        self.send_hci_cmd(&cmd)
    }

    pub fn create_connection(&mut self, addr: &BdAddr) -> BtResult {
        let _ = addr;
        Err(BtError::ConnectionFailed)
    }

    pub fn disconnect(&self, _reason: u8) -> BtResult {
        Ok(())
    }

    pub fn set_scan_params(&self) -> BtResult {
        let mut cmd = HciCommand {
            opcode: 0x200B,
            params: [0u8; 16],
            param_len: 7,
        };
        cmd.params[0] = 0x01;
        cmd.params[1] = 0x30;
        cmd.params[2] = 0x00;
        cmd.params[3] = 0x30;
        cmd.params[4] = 0x00;
        cmd.params[5] = 0x00;
        cmd.params[6] = 0x00;
        self.send_hci_cmd(&cmd)
    }
}
