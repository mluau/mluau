use std::os::raw::{c_int, c_void};

use crate::error::Result;
use crate::state::{Lua, RawLua};
use std::ops::Deref;
// Re-export mutex wrappers
pub use sync::XRc;
pub(crate) use sync::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XWeak};

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use either::Either;
pub(crate) use value_ref::ValueRef;

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

/// A reference to a value `T`
/// 
/// Required to hold the underlying Luau VM alive until reference is dropped
pub struct LuaRef<'a, T: 'static> {
    lua: Lua,     
    data: &'a T,
}

impl<'a, T: 'static> LuaRef<'a, T> {
    pub(crate) fn new(lua: Lua, data: &'a T) -> Self {
        Self { lua: lua, data }
    }

    pub(crate) fn new_opt(lua: Lua, data: Option<&'a T>) -> Option<Self> {
        match data {
            Some(data) => Some(Self::new(lua, data)),
            None => None
        }
    }

    /// Returns a reference to the Lua reference backing `T`
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

impl<'a, T: 'static> Deref for LuaRef<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.data
    }
}

#[repr(C)]
pub(crate) struct ErasedHeader {
    type_id: std::any::TypeId,
    drop_fn: unsafe fn(*mut std::ffi::c_void),
}

#[repr(C)]
struct ErasedWrapper<T> {
    header: ErasedHeader,
    data: T,
}

impl ErasedHeader {
    #[inline]
    /// Converts T into a ErasedWrapper pointer to ErasedWrapper<T>
    pub(crate) fn into_raw<T: 'static>(data: T) -> *mut std::ffi::c_void {
        let wrapper = Box::new(ErasedWrapper {
            header: ErasedHeader {
                type_id: std::any::TypeId::of::<T>(),
                drop_fn: |ptr| unsafe {
                    let _ = Box::from_raw(ptr as *mut ErasedWrapper<T>);
                },
            },
            data,
        });
        Box::into_raw(wrapper) as *mut std::ffi::c_void
    }

    /// Returns the exact number of bytes needed to store the wrapper in the GC heap.
    #[inline]
    pub(crate) const fn wrapper_size<T: 'static>() -> usize {
        std::mem::size_of::<ErasedWrapper<T>>()
    }

    /// Initializes uninitialized memory allocated by the Luau GC.
    /// 
    /// Note: Luau GC blocks are deallocated by GC itself so we use std::ptr::drop_in_place instead and avoid manual boxing
    #[inline]
    pub(crate) unsafe fn place_into_gc_memory<T: 'static>(ptr: *mut std::ffi::c_void, data: T) {
        std::ptr::write(ptr as *mut ErasedWrapper<T>, ErasedWrapper {
            header: ErasedHeader {
                type_id: std::any::TypeId::of::<T>(),
                drop_fn: |p| unsafe {
                    // SAFETY: Luau GC will handle freeing the actual memory block, we just need to drop_in_place
                    std::ptr::drop_in_place(p as *mut ErasedWrapper<T>);
                },
            },
            data,
        });
    }

    #[inline]
    pub(crate) unsafe fn downcast_ref<'a, T: 'static>(ptr: *const std::ffi::c_void) -> Option<&'a T> {
        if ptr.is_null() {
            return None;
        }
        let header = &*(ptr as *const ErasedHeader);
        if header.type_id == std::any::TypeId::of::<T>() {
            let wrapper = &*(ptr as *const ErasedWrapper<T>);
            Some(&wrapper.data)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) unsafe fn drop(ptr: *mut std::ffi::c_void) {
        if !ptr.is_null() {
            let drop_fn = (*(ptr as *const ErasedHeader)).drop_fn;
            drop_fn(ptr);
        }
    }
}

/// A "light" userdata value. Equivalent to an unmanaged raw pointer.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct LightUserData(pub *mut c_void);

#[cfg(feature = "send")]
unsafe impl Send for LightUserData {}
#[cfg(feature = "send")]
unsafe impl Sync for LightUserData {}

#[cfg(feature = "send")]
type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> std::result::Result<c_int, crate::Value> + Send + 'a;

#[cfg(not(feature = "send"))]
type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> std::result::Result<c_int, crate::Value> + 'a;

pub(crate) type Callback = Box<CallbackFn<'static>>;

#[cfg(feature = "send")]
pub(crate) type Continuation = Box<dyn Fn(&RawLua, c_int, c_int) -> std::result::Result<c_int, crate::Value> + Send + 'static>;
#[cfg(not(feature = "send"))]
pub(crate) type Continuation = Box<dyn Fn(&RawLua, c_int, c_int) -> std::result::Result<c_int, crate::Value> + 'static>;

/// Type to set next Lua VM action after executing interrupt or hook function.
pub enum VmState {
    Continue,
    /// Yield the current thread.
    ///
    /// Supported by Lua 5.3+ and Luau.
    Yield,
}


#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState>>;

pub(crate) type GcInterruptCallback = XRc<dyn Fn(&Lua, c_int) -> ()>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type ThreadCreationCallback = XRc<dyn Fn(&Lua, crate::Thread) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type ThreadCreationCallback = XRc<dyn Fn(&Lua, crate::Thread) -> Result<()>>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type ThreadCollectionCallback = XRc<dyn Fn(crate::LightUserData) + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type ThreadCollectionCallback = XRc<dyn Fn(crate::LightUserData)>;


/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(feature = "send")]
pub trait MaybeSend: Send {}
#[cfg(feature = "send")]
impl<T: Send> MaybeSend for T {}

/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(not(feature = "send"))]
pub trait MaybeSend {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSend for T {}

/// A trait that adds `Sync` requirement if `send` feature is enabled.
#[cfg(feature = "send")]
pub trait MaybeSync: Sync {}
#[cfg(feature = "send")]
impl<T: Sync> MaybeSync for T {}

/// A trait that adds `Sync` requirement if `send` feature is enabled.
#[cfg(not(feature = "send"))]
pub trait MaybeSync {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSync for T {}

pub(crate) trait LuaType {
    const TYPE_ID: c_int;
}

impl LuaType for bool {
    const TYPE_ID: c_int = ffi::LUA_TBOOLEAN;
}

impl LuaType for Number {
    const TYPE_ID: c_int = ffi::LUA_TNUMBER;
}

impl LuaType for LightUserData {
    const TYPE_ID: c_int = ffi::LUA_TLIGHTUSERDATA;
}

mod app_data;
mod sync;
mod value_ref;

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(ValueRef: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(ValueRef: Send, Sync);
}
