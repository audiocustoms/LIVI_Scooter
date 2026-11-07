// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

//! The two ways to reach the chip: the real I²C bus on Linux, and a fake that
//! answers from a register map so the driver can be tested without a board.

/// What can go wrong while talking to the chip.
#[derive(Debug)]
pub enum Error {
    /// The bus itself failed: no device, a short read, a rejected address.
    Bus(String),
    /// The register returned fewer bytes than its width needs.
    Short(u8, usize),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bus(e) => write!(f, "{e}"),
            Self::Short(reg, n) => write!(f, "register 0x{reg:02X}: fewer than {n} bytes"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A register level connection. The widths below are what the chip reports;
/// the INA228 holds 20 bit measurements in the top of three bytes, and 40 bit
/// accumulators in five.
pub trait Bus {
    fn read(&mut self, reg: u8, out: &mut [u8]) -> Result<()>;
    fn write(&mut self, reg: u8, data: &[u8]) -> Result<()>;

    fn read_u16(&mut self, reg: u8) -> Result<u16> {
        let mut b = [0u8; 2];
        self.read(reg, &mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    fn read_i16(&mut self, reg: u8) -> Result<i16> {
        let mut b = [0u8; 2];
        self.read(reg, &mut b)?;
        Ok(i16::from_be_bytes(b))
    }

    fn write_u16(&mut self, reg: u8, value: u16) -> Result<()> {
        self.write(reg, &value.to_be_bytes())
    }

    /// A 20 bit measurement, right aligned in three bytes.
    fn read_u24_shift4(&mut self, reg: u8) -> Result<u32> {
        let mut b = [0u8; 3];
        self.read(reg, &mut b)?;
        Ok(u32::from_be_bytes([0, b[0], b[1], b[2]]) >> 4)
    }

    /// The same, but signed, which current and shunt voltage are.
    fn read_i24_shift4(&mut self, reg: u8) -> Result<i32> {
        let raw = self.read_u24_shift4(reg)? as i32;
        Ok(if raw & (1 << 19) != 0 {
            raw - (1 << 20)
        } else {
            raw
        })
    }

    fn read_u40(&mut self, reg: u8) -> Result<u64> {
        let mut b = [0u8; 5];
        self.read(reg, &mut b)?;
        Ok(u64::from_be_bytes([0, 0, 0, b[0], b[1], b[2], b[3], b[4]]))
    }

    fn read_i40(&mut self, reg: u8) -> Result<i64> {
        let raw = self.read_u40(reg)? as i64;
        Ok(if raw & (1 << 39) != 0 {
            raw - (1 << 40)
        } else {
            raw
        })
    }
}

/// A bus that answers from a register map, for tests and for running the driver
/// before the board exists.
#[derive(Debug, Default, Clone)]
pub struct Fake {
    bytes: std::collections::BTreeMap<u8, Vec<u8>>,
    written: std::collections::BTreeMap<u8, u16>,
}

impl Fake {
    pub fn set_u16(&mut self, reg: u8, value: u16) {
        self.bytes.insert(reg, value.to_be_bytes().to_vec());
    }

    pub fn set_u24(&mut self, reg: u8, value: u32) {
        let b = value.to_be_bytes();
        self.bytes.insert(reg, b[1..4].to_vec());
    }

    pub fn set_i40(&mut self, reg: u8, value: i64) {
        let b = value.to_be_bytes();
        self.bytes.insert(reg, b[3..8].to_vec());
    }

    /// The last value written to a register, for tests that check the setup.
    pub fn written(&self, reg: u8) -> Option<u16> {
        self.written.get(&reg).copied()
    }
}

impl Bus for Fake {
    fn read(&mut self, reg: u8, out: &mut [u8]) -> Result<()> {
        // A real chip answers every register, so an unset one reads as zero.
        match self.bytes.get(&reg) {
            Some(bytes) if bytes.len() >= out.len() => out.copy_from_slice(&bytes[..out.len()]),
            Some(_) => return Err(Error::Short(reg, out.len())),
            None => out.fill(0),
        }
        Ok(())
    }

    fn write(&mut self, reg: u8, data: &[u8]) -> Result<()> {
        if data.len() == 2 {
            self.written
                .insert(reg, u16::from_be_bytes([data[0], data[1]]));
        }
        Ok(())
    }
}

impl<T: Bus + ?Sized> Bus for &mut T {
    fn read(&mut self, reg: u8, out: &mut [u8]) -> Result<()> {
        (**self).read(reg, out)
    }

    fn write(&mut self, reg: u8, data: &[u8]) -> Result<()> {
        (**self).write(reg, data)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::os::fd::AsRawFd;

    use super::{Bus, Error, Result};

    /// Address of a register select before reading or writing.
    const I2C_SLAVE: libc::c_ulong = 0x0703;

    /// The INA228 on a Pi's I²C bus.
    pub struct LinuxI2c {
        file: fs::File,
    }

    impl LinuxI2c {
        pub fn open(bus: u32, address: u16) -> Result<Self> {
            let path = format!("/dev/i2c-{bus}");
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| Error::Bus(format!("open {path}: {e}")))?;
            let rc =
                unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, address as libc::c_ulong) };
            if rc < 0 {
                return Err(Error::Bus(format!(
                    "I2C_SLAVE 0x{address:02X} on {path}: {}",
                    std::io::Error::last_os_error()
                )));
            }
            Ok(Self { file })
        }
    }

    impl Bus for LinuxI2c {
        fn read(&mut self, reg: u8, out: &mut [u8]) -> Result<()> {
            use std::io::{Read, Write};
            self.file
                .write_all(&[reg])
                .map_err(|e| Error::Bus(format!("select 0x{reg:02X}: {e}")))?;
            self.file
                .read_exact(out)
                .map_err(|e| Error::Bus(format!("read 0x{reg:02X}: {e}")))
        }

        fn write(&mut self, reg: u8, data: &[u8]) -> Result<()> {
            use std::io::Write;
            let mut frame = Vec::with_capacity(data.len() + 1);
            frame.push(reg);
            frame.extend_from_slice(data);
            self.file
                .write_all(&frame)
                .map_err(|e| Error::Bus(format!("write 0x{reg:02X}: {e}")))
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::LinuxI2c;
