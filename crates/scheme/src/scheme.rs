use core::slice;

use crate::Stat;
use crate::{str_from_raw_parts, Packet};
use axerrno::to_ret_code;
use axerrno::AxError as Error;
use axerrno::AxResult as Result;
use core::result::Result::Err;

/// Scheme: abstraction of service handlers
/// From <https://gitlab.redox-os.org/redox-os/syscall/-/blob/master/src/scheme/scheme.rs>
pub trait Scheme {
    /// Do not reimplement. dispatch all `Packet`s to different handlers.
    fn handle(&self, packet: &mut Packet) {
        use syscall_number::*;
        let res = match packet.a {
            SYS_OPEN => {
                if let Some(path) = unsafe { str_from_raw_parts(packet.b as *const u8, packet.c) } {
                    self.open(path, packet.d, packet.uid, packet.gid)
                } else {
                    Err(Error::InvalidData)
                }
            }
            SYS_RMDIR => {
                if let Some(path) = unsafe { str_from_raw_parts(packet.b as *const u8, packet.c) } {
                    self.rmdir(path, packet.uid, packet.gid)
                } else {
                    Err(Error::InvalidData)
                }
            }
            SYS_UNLINK => {
                if let Some(path) = unsafe { str_from_raw_parts(packet.b as *const u8, packet.c) } {
                    self.unlink(path, packet.uid, packet.gid)
                } else {
                    Err(Error::InvalidData)
                }
            }

            SYS_DUP => self.dup(packet.b, unsafe {
                slice::from_raw_parts(packet.c as *const u8, packet.d)
            }),
            SYS_READ => self.read(packet.b, unsafe {
                slice::from_raw_parts_mut(packet.c as *mut u8, packet.d)
            }),
            SYS_WRITE => self.write(packet.b, unsafe {
                slice::from_raw_parts(packet.c as *const u8, packet.d)
            }),
            SYS_LSEEK => self
                .seek(packet.b, packet.c as isize, packet.d)
                .map(|o| o as usize),
            SYS_FCHMOD => self.fchmod(packet.b, packet.c as u16),
            SYS_FCHOWN => self.fchown(packet.b, packet.c as u32, packet.d as u32),
            SYS_FCNTL => self.fcntl(packet.b, packet.c, packet.d),
            // SYS_FEVENT => self.fevent(packet.b, EventFlags::from_bits_truncate(packet.c)).map(|f| f.bits()),
            // SYS_FMAP_OLD => if packet.d >= mem::size_of::<OldMap>() {
            //     self.fmap_old(packet.b, unsafe { &*(packet.c as *const OldMap) })
            // } else {
            //     Err(Error::new(EFAULT))
            // },
            // SYS_FMAP => if packet.d >= mem::size_of::<Map>() {
            //     self.fmap(packet.b, unsafe { &*(packet.c as *const Map) })
            // } else {
            //     Err(Error::new(EFAULT))
            // },
            // SYS_FUNMAP_OLD => self.funmap_old(packet.b),
            // SYS_FUNMAP => self.funmap(packet.b, packet.c),
            SYS_FPATH => self.fpath(packet.b, unsafe {
                slice::from_raw_parts_mut(packet.c as *mut u8, packet.d)
            }),
            SYS_FRENAME => {
                if let Some(path) = unsafe { str_from_raw_parts(packet.c as *const u8, packet.d) } {
                    self.frename(packet.b, path, packet.uid, packet.gid)
                } else {
                    Err(Error::InvalidData)
                }
            }
            SYS_FSTAT => {
                if packet.d >= core::mem::size_of::<Stat>() {
                    self.fstat(packet.b, unsafe { &mut *(packet.c as *mut Stat) })
                } else {
                    Err(Error::BadAddress)
                }
            }
            // SYS_FSTATVFS => if packet.d >= mem::size_of::<StatVfs>() {
            //     self.fstatvfs(packet.b, unsafe { &mut *(packet.c as *mut StatVfs) })
            // } else {
            //     Err(Error::BadAddress)
            // },
            SYS_FSYNC => self.fsync(packet.b),
            SYS_FTRUNCATE => self.ftruncate(packet.b, packet.c),
            // SYS_FUTIMENS => if packet.d >= mem::size_of::<TimeSpec>() {
            //     self.futimens(packet.b, unsafe { slice::from_raw_parts(packet.c as *const TimeSpec, packet.d / mem::size_of::<TimeSpec>()) })
            // } else {
            //     Err(Error::BadAddress)
            // },
            SYS_CLOSE => self.close(packet.b),
            _ => Err(Error::BadFileDescriptor),
        };

        packet.a = to_ret_code(res) as usize;
    }

    /* Scheme operations */

    /// `open` syscall
    #[allow(unused_variables)]
    fn open(&self, path: &str, flags: usize, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::NotFound)
    }

    /// `chmod` syscall
    #[allow(unused_variables)]
    fn chmod(&self, path: &str, mode: u16, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::NotFound)
    }

    /// `rmdir` syscall
    #[allow(unused_variables)]
    fn rmdir(&self, path: &str, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::NotFound)
    }

    /// `unlink` syscall
    #[allow(unused_variables)]
    fn unlink(&self, path: &str, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::NotFound)
    }

    /// `dup` syscall
    /// if `buf` is empty, normal `dup` operation (duplicate `fd`s) is done,
    /// otherwise, `Scheme`-defined operations are done.
    /* Resource operations */
    #[allow(unused_variables)]
    fn dup(&self, old_id: usize, buf: &[u8]) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `read` syscall
    #[allow(unused_variables)]
    fn read(&self, id: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `write` syscall
    #[allow(unused_variables)]
    fn write(&self, id: usize, buf: &[u8]) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `seek` syscall
    #[allow(unused_variables)]
    fn seek(&self, id: usize, pos: isize, whence: usize) -> Result<isize> {
        Err(Error::BadFileDescriptor)
    }

    /// `fchmod` syscall
    #[allow(unused_variables)]
    fn fchmod(&self, id: usize, mode: u16) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `fchown` syscall
    #[allow(unused_variables)]
    fn fchown(&self, id: usize, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `fcntl` syscall
    #[allow(unused_variables)]
    fn fcntl(&self, id: usize, cmd: usize, arg: usize) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }
    /*
       #[allow(unused_variables)]
       fn fevent(&self, id: usize, flags: EventFlags) -> Result<EventFlags> {
           Err(Error::BadFileDescriptor)
       }

       #[allow(unused_variables)]
       fn fmap_old(&self, id: usize, map: &OldMap) -> Result<usize> {
           Err(Error::BadFileDescriptor)
       }
       #[allow(unused_variables)]
       fn fmap(&self, id: usize, map: &Map) -> Result<usize> {
           if map.flags.contains(MapFlags::MAP_FIXED) {
               return Err(Error::new(EINVAL));
           }
           self.fmap_old(id, &OldMap {
               offset: map.offset,
               size: map.size,
               flags: map.flags,
           })
       }

       #[allow(unused_variables)]
       fn funmap_old(&self, address: usize) -> Result<usize> {
           Ok(0)
       }

       #[allow(unused_variables)]
       fn funmap(&self, address: usize, length: usize) -> Result<usize> {
           Ok(0)
       }
    */

    /// `fpath` syscall
    #[allow(unused_variables)]
    fn fpath(&self, id: usize, buf: &mut [u8]) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `frename` syscall
    #[allow(unused_variables)]
    fn frename(&self, id: usize, path: &str, uid: u32, gid: u32) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `fstat` syscall
    #[allow(unused_variables)]
    fn fstat(&self, id: usize, stat: &mut Stat) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }
    /*
       #[allow(unused_variables)]
       fn fstatvfs(&self, id: usize, stat: &mut StatVfs) -> Result<usize> {
           Err(Error::BadFileDescriptor)
       }
    */

    /// `fsync` syscall
    #[allow(unused_variables)]
    fn fsync(&self, id: usize) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }

    /// `ftruncate` syscall
    #[allow(unused_variables)]
    fn ftruncate(&self, id: usize, len: usize) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }
    /*
       #[allow(unused_variables)]
       fn futimens(&self, id: usize, times: &[TimeSpec]) -> Result<usize> {
           Err(Error::BadFileDescriptor)
       }
    */

    /// `close` syscall
    #[allow(unused_variables)]
    fn close(&self, id: usize) -> Result<usize> {
        Err(Error::BadFileDescriptor)
    }
}
