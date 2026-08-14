use std::io;
use std::os::raw::c_void;

#[cfg(feature = "serde")]
use serde::ser::{Serialize, Serializer};

use crate::state::RawLua;
use crate::types::{TypedRef, ValueRef};

/// A Luau buffer type.
///
/// See the buffer [documentation] for more information.
///
/// [documentation]: https://luau.org/library#buffer-library
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug, PartialEq)]
pub struct Buffer(pub(crate) ValueRef);


impl Buffer {
    /// Copies the buffer data into a new `Vec<u8>`.
    pub fn to_vec(&self) -> Vec<u8> {
        let lua = self.0.lua.lock();
        self.as_slice(&lua).to_vec()
    }

    /// Calls a function f with the byte slice of the buffer.
    ///
    /// Safety: The byte slice must not outlive the buffer.
    pub fn with_bytes<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let lua = self.0.lua.lock();
        let data = self.as_slice(&lua);
        f(data)
    }

    /// Calls a function f with the byte slice of the buffer.
    ///
    /// Safety: The byte slice must not outlive the buffer.
    pub async fn with_bytes_async<F, R>(&self, f: F) -> R
    where
        F: AsyncFnOnce(&[u8]) -> R,
    {
        let lua = self.0.lua.lock();
        let data = self.as_slice(&lua);
        f(data).await
    }

    /// Returns the length of the buffer.
    pub fn len(&self) -> usize {
        let lua = self.0.lua.lock();
        self.as_slice(&lua).len()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Downcasts the external buffer to the specified backing store type.
    /// 
    /// # Safety:
    /// 
    /// Assumes the external buffer was created by mluau's create_external_buffer
    pub fn downcast_ref<T: ExternalBuffer>(&self) -> Option<TypedRef<T, Self, 0>> { // TODO: when we get tagged buffers, use the tag here
        let lua = self.0.lua.lock();
        let state = lua.state();
        let ptr = unsafe { 
            let _sg = crate::util::StackGuard::new(state);
            lua.push_ref_at(&self.0, state);
            let ud = ffi::lua_getbufferuserdata(state, -1);
            crate::types::ErasedHeader::downcast_ref(ud)
        };
        TypedRef::new_opt(lua.0, ptr, self.clone())
    }

    /// Reads given number of bytes from the buffer at the given offset.
    ///
    /// Offset is 0-based.
    #[track_caller]
    pub fn read_bytes<const N: usize>(&self, offset: usize) -> [u8; N] {
        let lua = self.0.lua.lock();
        let data = self.as_slice(&lua);
        let mut bytes = [0u8; N];
        bytes.copy_from_slice(&data[offset..offset + N]);
        bytes
    }

    /// Reads given number of bytes from the buffer at the given offset.
    ///
    /// Offset is 0-based.
    ///
    /// Unlike read_bytes, this function returns a vector of bytes and is
    /// not generic over the number of bytes.
    #[track_caller]
    pub fn read_bytes_to_vec(&self, offset: usize, len: usize) -> Vec<u8> {
        let lua = self.0.lua.lock();
        let data = self.as_slice(&lua);
        let mut bytes = vec![0u8; len];
        bytes.copy_from_slice(&data[offset..offset + len]);
        bytes
    }

    /// Writes given bytes to the buffer at the given offset.
    ///
    /// Offset is 0-based.
    #[track_caller]
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) {
        let lua = self.0.lua.lock();
        let data = self.as_slice_mut(&lua);
        data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// Returns an adaptor implementing [`io::Read`], [`io::Write`] and [`io::Seek`] over the
    /// buffer.
    ///
    /// Buffer operations are infallible, none of the read/write functions will return a Err.
    pub fn cursor(self) -> impl io::Read + io::Write + io::Seek {
        BufferCursor(self, 0)
    }

    pub(crate) fn as_slice(&self, lua: &RawLua) -> &[u8] {
        unsafe {
            let (buf, size) = self.as_raw_parts(lua);
            std::slice::from_raw_parts(buf, size)
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn as_slice_mut(&self, lua: &RawLua) -> &mut [u8] {
        unsafe {
            let (buf, size) = self.as_raw_parts(lua);
            std::slice::from_raw_parts_mut(buf, size)
        }
    }

    unsafe fn as_raw_parts(&self, lua: &RawLua) -> (*mut u8, usize) {
        let mut size = 0usize;
        let state = lua.state();
        let _sg = crate::util::StackGuard::new(state);
        lua.push_ref_at(&self.0, state);
        let buf = ffi::lua_tobuffer(state, -1, &mut size);
        mlua_assert!(!buf.is_null(), "invalid Luau buffer");
        (buf as *mut u8, size)
    }

    /// Converts this buffer to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }
}

struct BufferCursor(Buffer, usize);

impl io::Read for BufferCursor {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice(&lua);
        if self.1 == data.len() {
            return Ok(0);
        }
        let len = buf.len().min(data.len() - self.1);
        buf[..len].copy_from_slice(&data[self.1..self.1 + len]);
        self.1 += len;
        Ok(len)
    }
}

impl io::Write for BufferCursor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice_mut(&lua);
        if self.1 == data.len() {
            return Ok(0);
        }
        let len = buf.len().min(data.len() - self.1);
        data[self.1..self.1 + len].copy_from_slice(&buf[..len]);
        self.1 += len;
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl io::Seek for BufferCursor {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let lua = self.0 .0.lua.lock();
        let data = self.0.as_slice(&lua);
        let new_offset = match pos {
            io::SeekFrom::Start(offset) => offset as i64,
            io::SeekFrom::End(offset) => data.len() as i64 + offset,
            io::SeekFrom::Current(offset) => self.1 as i64 + offset,
        };
        if new_offset < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative position",
            ));
        }
        if new_offset as usize > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a position beyond the end of the buffer",
            ));
        }
        self.1 = new_offset as usize;
        Ok(self.1 as u64)
    }
}

#[cfg(feature = "serde")]
impl Serialize for Buffer {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let lua = self.0.lua.lock();
        serializer.serialize_bytes(self.as_slice(&lua))
    }
}

impl crate::types::LuaType for Buffer {
    const TYPE_ID: std::os::raw::c_int = ffi::LUA_TBUFFER;
}

/// A backing store for an externally managed Luau buffer.
///
/// # Safety
/// The implementor must ensure the memory returned by `as_ptr` and `len` remains valid
/// and safe to be read by the Luau VM for the lifetime of this object. Note that implementing
/// this trait by itself does not require the memory to be safe for mutation by the Luau VM;
/// mutability safety is only a concern if the type also implements `ExternalBufferMut`.
pub unsafe trait ExternalBuffer: 'static {
    /// Returns a pointer to the buffer data.
    fn as_ptr(&self) -> *const u8;
    /// Returns the length of the buffer.
    fn len(&self) -> usize;
}

/// A backing store for an externally managed, mutable Luau buffer.
///
/// # Safety
/// The implementor must ensure that the memory is safe to be mutated directly
/// by the Luau VM without causing undefined behavior in Rust.
pub unsafe trait ExternalBufferMut: ExternalBuffer {
    /// Returns a mutable pointer to the buffer data.
    fn as_mut_ptr(&mut self) -> *mut u8;
}

/// A marker trait for primitive types that are safe to be treated as raw bytes and mutated
/// by the Luau VM. Types implementing this must not contain references, padding bytes with
/// undefined behavior, or complex drop logic. Crucially, any arbitrary bit pattern must
/// represent a valid instance of the type without causing undefined behavior.
pub unsafe trait Primitive {}

unsafe impl Primitive for u8 {}
unsafe impl Primitive for i8 {}
unsafe impl Primitive for u16 {}
unsafe impl Primitive for i16 {}
unsafe impl Primitive for u32 {}
unsafe impl Primitive for i32 {}
unsafe impl Primitive for u64 {}
unsafe impl Primitive for i64 {}
unsafe impl Primitive for u128 {}
unsafe impl Primitive for i128 {}
unsafe impl Primitive for f32 {}
unsafe impl Primitive for f64 {}



// SAFETY: `Vec<T>` manages a heap allocation that will not move or be deallocated
// as long as the `Vec` itself is alive. The pointer and length returned are valid
// to safely read from.
unsafe impl<T: 'static> ExternalBuffer for Vec<T> {
    fn as_ptr(&self) -> *const u8 {
        self.as_slice().as_ptr() as *const u8
    }

    fn len(&self) -> usize {
        self.as_slice().len() * std::mem::size_of::<T>()
    }
}

// SAFETY: `Vec<T>` where `T: Primitive` contains only plain bytes. Mutating it directly from
// the Luau VM is safe because any arbitrary bit pattern is a valid instance of `T` (or at least
// mutating it via VM won't cause UB during drops or pointer derefs).
unsafe impl<T: Primitive + 'static> ExternalBufferMut for Vec<T> {
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_slice().as_mut_ptr() as *mut u8
    }
}

// SAFETY: `Arc<T>` is a reference-counted pointer to `T`. The underlying memory
// allocation managed by `T` is stable and valid for reading as long as the `Arc` is alive.
//
// Thread Safety: `Arc` provides thread-safe reference counting, making it perfectly safe
// if the Luau VM's garbage collector drops this object from a different thread.
unsafe impl<T: ExternalBuffer> ExternalBuffer for std::sync::Arc<T> {
    fn as_ptr(&self) -> *const u8 {
        (**self).as_ptr()
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}

// SAFETY: `Rc<T>` is a reference-counted pointer to `T`. The underlying memory
// allocation managed by `T` is stable and valid for reading as long as the `Rc` is alive.
unsafe impl<T: ExternalBuffer> ExternalBuffer for std::rc::Rc<T> {
    fn as_ptr(&self) -> *const u8 {
        (**self).as_ptr()
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}

#[cfg(feature = "bytes")]
// SAFETY: `bytes::Bytes` is an immutable, reference-counted contiguous byte slice.
// Its memory allocation is stable and guaranteed valid for reading.
unsafe impl ExternalBuffer for bytes::Bytes {
    fn as_ptr(&self) -> *const u8 {
        self.as_ref().as_ptr()
    }

    fn len(&self) -> usize {
        self.as_ref().len()
    }
}

#[cfg(feature = "bytes")]
// SAFETY: `bytes::BytesMut` guarantees contiguous memory.
// Its pointer and length remain valid for reading for the lifetime of the object.
unsafe impl ExternalBuffer for bytes::BytesMut {
    fn as_ptr(&self) -> *const u8 {
        self.as_ref().as_ptr()
    }

    fn len(&self) -> usize {
        self.as_ref().len()
    }
}

#[cfg(feature = "bytes")]
// SAFETY: `bytes::BytesMut` contains only plain bytes. Mutating it directly from the Luau VM
// is safe because any arbitrary bit pattern is a valid `u8` and it doesn't cause UB.
unsafe impl ExternalBufferMut for bytes::BytesMut {
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut().as_mut_ptr()
    }
}
