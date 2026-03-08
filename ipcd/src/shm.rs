use redox_scheme::{scheme::SchemeSync, CallerCtx, OpenResult};
use std::{
    cmp,
    collections::{hash_map::Entry, HashMap},
    rc::Rc,
};
use syscall::{
    data::Stat, error::*, schemev2::NewFdFlags, Error, Map, MapFlags, Result, MAP_PRIVATE,
    PAGE_SIZE, PROT_READ, PROT_WRITE,
};

enum Handle {
    Shm(Rc<str>),
    SchemeRoot,
}
impl Handle {
    fn as_shm(&self) -> Option<&Rc<str>> {
        match self {
            Self::Shm(path) => Some(path),
            Self::SchemeRoot => None,
        }
    }
}

// TODO: Move to relibc
const AT_REMOVEDIR: usize = 0x200;

pub struct ShmHandle {
    buffer: MmapGuard,
    refs: usize,
    unlinked: bool,
}
pub struct ShmScheme {
    maps: HashMap<Rc<str>, ShmHandle>,
    handles: HashMap<usize, Handle>,
    next_id: usize,
}
impl ShmScheme {
    pub fn new() -> Self {
        Self {
            maps: HashMap::new(),
            handles: HashMap::new(),
            next_id: 0,
        }
    }
}

impl SchemeSync for ShmScheme {
    fn scheme_root(&mut self) -> Result<usize> {
        let id = self.next_id;
        self.next_id += 1;

        self.handles.insert(id, Handle::SchemeRoot);
        Ok(id)
    }
    //FIXME: Handle O_RDONLY/O_WRONLY/O_RDWR
    fn openat(
        &mut self,
        dirfd: usize,
        path: &str,
        flags: usize,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<OpenResult> {
        let handle = self.handles.get(&dirfd).ok_or(Error::new(EBADF))?;
        if !matches!(handle, Handle::SchemeRoot) {
            return Err(Error::new(EACCES));
        }

        let path = Rc::from(path);
        let entry = match self.maps.entry(Rc::clone(&path)) {
            Entry::Occupied(e) => {
                if flags & syscall::O_EXCL != 0 && flags & syscall::O_CREAT != 0 {
                    return Err(Error::new(EEXIST));
                }
                e.into_mut()
            }
            Entry::Vacant(e) => {
                if flags & syscall::O_CREAT == 0 {
                    return Err(Error::new(ENOENT));
                }
                e.insert(ShmHandle {
                    buffer: MmapGuard::new(),
                    refs: 0,
                    unlinked: false,
                })
            }
        };
        entry.refs += 1;
        self.handles.insert(self.next_id, Handle::Shm(path));

        let id = self.next_id;
        self.next_id += 1;
        Ok(OpenResult::ThisScheme {
            number: id,
            flags: NewFdFlags::POSITIONED,
        })
    }
    fn fpath(&mut self, id: usize, buf: &mut [u8], _ctx: &CallerCtx) -> Result<usize> {
        // Write scheme name
        const PREFIX: &[u8] = b"/scheme/shm/";
        let len = cmp::min(PREFIX.len(), buf.len());
        buf[..len].copy_from_slice(&PREFIX[..len]);
        if len < PREFIX.len() {
            return Ok(len);
        }

        // Write path
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        let len = cmp::min(path.len(), buf.len() - PREFIX.len());
        buf[PREFIX.len()..][..len].copy_from_slice(&path.as_bytes()[..len]);

        Ok(PREFIX.len() + len)
    }
    fn on_close(&mut self, id: usize) {
        let Handle::Shm(path) = self.handles.remove(&id).unwrap() else {
            return;
        };
        let mut entry = match self.maps.entry(path) {
            Entry::Occupied(entry) => entry,
            Entry::Vacant(_) => panic!("handle pointing to nothing"),
        };
        entry.get_mut().refs -= 1;
        if entry.get().refs == 0 && entry.get().unlinked {
            // There is no other reference to this entry, drop
            entry.remove_entry();
        }
    }
    fn unlinkat(&mut self, dirfd: usize, path: &str, flags: usize, _ctx: &CallerCtx) -> Result<()> {
        let handle = self.handles.get(&dirfd).ok_or(Error::new(EBADF))?;
        if !matches!(handle, Handle::SchemeRoot) {
            return Err(Error::new(EACCES));
        }
        if flags & AT_REMOVEDIR == AT_REMOVEDIR {
            return Err(Error::new(ENOTDIR));
        }
        let path = Rc::from(path);
        let mut entry = match self.maps.entry(Rc::clone(&path)) {
            Entry::Occupied(e) => e,
            Entry::Vacant(_) => return Err(Error::new(ENOENT)),
        };

        entry.get_mut().unlinked = true;
        if entry.get().refs == 0 {
            // There is no other reference to this entry, drop
            entry.remove_entry();
        }
        Ok(())
    }
    fn fstat(&mut self, id: usize, stat: &mut Stat, _ctx: &CallerCtx) -> Result<()> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        let size = self
            .maps
            .get(path)
            .expect("handle pointing to nothing")
            .buffer
            .len();

        //TODO: fill in more items?
        *stat = Stat {
            st_mode: syscall::MODE_FILE,
            st_size: size as _,
            ..Default::default()
        };

        Ok(())
    }
    fn fsize(&mut self, id: usize, _ctx: &CallerCtx) -> Result<u64> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        let size = self
            .maps
            .get(path)
            .expect("handle pointing to nothing")
            .buffer
            .len();

        Ok(size as u64)
    }
    fn ftruncate(&mut self, id: usize, len: u64, _ctx: &CallerCtx) -> Result<()> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        self.maps
            .get_mut(path)
            .expect("handle pointing to nothing")
            .buffer
            .grow_to(len as usize)
    }
    fn mmap_prep(
        &mut self,
        id: usize,
        offset: u64,
        size: usize,
        _flags: MapFlags,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        self.maps
            .get_mut(path)
            .expect("handle pointing to nothing")
            .buffer
            .mmap(offset as usize, size)
    }
    fn read(
        &mut self,
        id: usize,
        buf: &mut [u8],
        offset: u64,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        self.maps
            .get_mut(path)
            .expect("handle pointing to nothing")
            .buffer
            .read(offset as usize, buf)
    }
    fn write(
        &mut self,
        id: usize,
        buf: &[u8],
        offset: u64,
        _fcntl_flags: u32,
        _ctx: &CallerCtx,
    ) -> Result<usize> {
        let path = self
            .handles
            .get(&id)
            .and_then(Handle::as_shm)
            .ok_or(Error::new(EBADF))?;
        self.maps
            .get_mut(path)
            .expect("handle pointing to nothing")
            .buffer
            .write(offset as usize, buf)
    }
}

pub struct MmapSegment {
    base: usize,
    size: usize,
}

pub struct MmapGuard {
    segments: Vec<MmapSegment>,
    len: usize,
}

impl MmapGuard {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            len: 0,
        }
    }

    fn grow_to(&mut self, new_len: usize) -> Result<()> {
        if new_len <= self.total_capacity() {
            // FIXME clear bytes after new_len
            self.len = new_len;
            return Ok(());
        }

        let needed = new_len - self.total_capacity();
        let page_count = needed.div_ceil(PAGE_SIZE);
        let alloc_size = page_count * PAGE_SIZE;

        let base = unsafe {
            syscall::fmap(
                !0,
                &Map {
                    offset: 0,
                    size: alloc_size,
                    flags: MAP_PRIVATE | PROT_READ | PROT_WRITE,
                    address: 0,
                },
            )
        }?;

        self.segments.push(MmapSegment {
            base,
            size: alloc_size,
        });
        self.len = new_len;
        Ok(())
    }

    fn total_capacity(&self) -> usize {
        self.segments.iter().map(|s| s.size).sum()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn mmap(&mut self, offset: usize, size: usize) -> Result<usize> {
        let total_size = offset + size;

        if total_size
            > self
                .segments
                .iter()
                .map(|segment| segment.size)
                .sum::<usize>()
        {
            return Err(Error::new(ERANGE));
        }

        if size == 0 {
            return Ok(0);
        }

        let mut seg_offset = 0;
        for segment in &self.segments {
            if offset > seg_offset + segment.size {
                seg_offset += segment.size;
                continue;
            }

            if total_size > seg_offset + segment.size {
                // Crossing two segments
                // TODO: Handle mmap that cross segments. Likely needs kernel support.
                eprintln!("shm: mmap across backing segments not supported");
                return Err(Error::new(EINVAL));
            }

            return Ok(segment.base + (offset - seg_offset));
        }

        unreachable!()
    }

    pub fn read(&self, mut offset: usize, buf: &mut [u8]) -> Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }

        let mut bytes_read = 0;
        let mut buf_idx = 0;
        let to_read = cmp::min(buf.len(), self.len - offset);

        for seg in &self.segments {
            if offset < seg.size {
                let chunk_size = cmp::min(seg.size - offset, to_read - bytes_read);
                unsafe {
                    let src = (seg.base as *const u8).add(offset);
                    let dst = buf.as_mut_ptr().add(buf_idx);
                    core::ptr::copy_nonoverlapping(src, dst, chunk_size);
                }
                bytes_read += chunk_size;
                buf_idx += chunk_size;
                offset = 0;
            } else {
                offset -= seg.size;
            }
            if bytes_read >= to_read {
                break;
            }
        }

        Ok(bytes_read)
    }

    pub fn write(&mut self, offset: usize, buf: &[u8]) -> Result<usize> {
        let end = offset.checked_add(buf.len()).ok_or(Error::new(ERANGE))?;

        if end > self.total_capacity() {
            self.grow_to(end)?;
        } else if end > self.len {
            self.len = end;
        }

        let mut bytes_written = 0;
        let mut current_offset = offset;

        for seg in &self.segments {
            if current_offset < seg.size {
                let chunk_size = cmp::min(seg.size - current_offset, buf.len() - bytes_written);
                unsafe {
                    let src = buf.as_ptr().add(bytes_written);
                    let dst = (seg.base as *mut u8).add(current_offset);
                    core::ptr::copy_nonoverlapping(src, dst, chunk_size);
                }
                bytes_written += chunk_size;
                current_offset = 0;
            } else {
                current_offset -= seg.size;
            }
            if bytes_written >= buf.len() {
                break;
            }
        }

        Ok(bytes_written)
    }
}

impl Drop for MmapGuard {
    fn drop(&mut self) {
        for seg in &self.segments {
            let _ = unsafe { syscall::funmap(seg.base, seg.size) };
        }
    }
}
