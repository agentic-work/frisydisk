//! macOS directory reader built on `getattrlistbulk`, which returns names,
//! types and allocated sizes for a whole directory in a few system calls.

use super::{Bounds, RawEntry, ReadError};
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const VREG: u32 = 1;
const VDIR: u32 = 2;
const VLNK: u32 = 5;
const ATTR_CMN_ERROR: libc::attrgroup_t = 0x2000_0000;

const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;

extern "C" {
    fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
}

const BUF_SIZE: usize = 256 * 1024;

/// When scanning `/`, user data lives on the Data volume and is reached
/// through firmlinks, so that volume is allowed too. Its own mount point is
/// skipped so nothing is counted twice.
pub(crate) fn bounds(root: &Path) -> Bounds {
    let mut devices = Vec::new();
    if let Some(d) = device_of(root) {
        devices.push(d);
    }
    let mut skip_paths = Vec::new();
    if root == Path::new("/") {
        let data = PathBuf::from("/System/Volumes/Data");
        if let Some(d) = device_of(&data) {
            devices.push(d);
            skip_paths.push(data);
        }
    }
    Bounds { devices, skip_paths }
}

fn device_of(p: &Path) -> Option<u64> {
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::stat(c.as_ptr(), &mut st) } == 0).then_some(st.st_dev as u64)
}

pub(crate) struct Reader {
    buf: Vec<u64>,
}

impl Reader {
    pub fn new() -> Reader {
        // Never pull cloud-only (dataless) directories down just to measure them.
        unsafe {
            setiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_THREAD,
                IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
            );
        }
        Reader { buf: vec![0u64; BUF_SIZE / 8] }
    }

    pub fn read(&mut self, path: &Path, bounds: &Bounds) -> Result<Vec<RawEntry>, ReadError> {
        if bounds.skip_paths.iter().any(|p| p == path) {
            return Err(ReadError::Skip);
        }
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| ReadError::Skip)?;
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW) };
        if fd < 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // EDEADLK: a cloud-only directory we refused to materialise.
            return Err(if err == libc::EDEADLK || err == libc::ENOENT { ReadError::Skip } else { ReadError::Denied });
        }
        let result = self.read_fd(fd, bounds);
        unsafe { libc::close(fd) };
        result
    }

    fn read_fd(&mut self, fd: libc::c_int, bounds: &Bounds) -> Result<Vec<RawEntry>, ReadError> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } != 0 || !bounds.devices.contains(&(st.st_dev as u64)) {
            return Err(ReadError::Skip);
        }
        let dev = st.st_dev as u64;

        let mut attrs: libc::attrlist = unsafe { std::mem::zeroed() };
        attrs.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
        attrs.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
            | libc::ATTR_CMN_NAME
            | ATTR_CMN_ERROR
            | libc::ATTR_CMN_OBJTYPE
            | libc::ATTR_CMN_FILEID;
        attrs.fileattr = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE;

        let mut out = Vec::new();
        let base = self.buf.as_mut_ptr() as *mut u8;
        loop {
            let n = unsafe {
                libc::getattrlistbulk(fd, &mut attrs as *mut _ as *mut libc::c_void, base as *mut libc::c_void, BUF_SIZE, 0)
            };
            if n < 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                break;
            }
            if n == 0 {
                break;
            }
            let mut entry = base as *const u8;
            for _ in 0..n {
                // Each record: u32 length, attribute_set_t, then the attributes
                // that were returned, in bit order.
                unsafe {
                    let length = read::<u32>(entry) as usize;
                    let mut field = entry.add(4);
                    let returned = read::<libc::attribute_set_t>(field);
                    field = field.add(std::mem::size_of::<libc::attribute_set_t>());

                    let mut error = 0u32;
                    if returned.commonattr & ATTR_CMN_ERROR != 0 {
                        error = read::<u32>(field);
                        field = field.add(4);
                    }
                    let mut name = String::new();
                    if returned.commonattr & libc::ATTR_CMN_NAME != 0 {
                        let r = read::<libc::attrreference_t>(field);
                        let ptr = field.offset(r.attr_dataoffset as isize) as *const libc::c_char;
                        name = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                        field = field.add(std::mem::size_of::<libc::attrreference_t>());
                    }
                    let mut obj_type = 0u32;
                    if returned.commonattr & libc::ATTR_CMN_OBJTYPE != 0 {
                        obj_type = read::<u32>(field);
                        field = field.add(4);
                    }
                    let mut file_id = 0u64;
                    if returned.commonattr & libc::ATTR_CMN_FILEID != 0 {
                        file_id = read::<u64>(field);
                        field = field.add(8);
                    }
                    let mut link_count = 1u32;
                    if returned.fileattr & libc::ATTR_FILE_LINKCOUNT != 0 {
                        link_count = read::<u32>(field);
                        field = field.add(4);
                    }
                    let mut alloc = 0i64;
                    if returned.fileattr & libc::ATTR_FILE_ALLOCSIZE != 0 {
                        alloc = read::<i64>(field);
                    }
                    entry = entry.add(length);

                    if name.is_empty() || error != 0 {
                        continue;
                    }
                    let is_dir = obj_type == VDIR;
                    out.push(RawEntry {
                        name,
                        is_dir,
                        is_symlink: obj_type == VLNK,
                        size: if is_dir { 0 } else { alloc.max(0) as u64 },
                        link: (link_count > 1 && obj_type == VREG).then_some((dev, file_id)),
                    });
                }
            }
        }
        Ok(out)
    }
}

unsafe fn read<T: Copy>(p: *const u8) -> T {
    std::ptr::read_unaligned(p as *const T)
}
