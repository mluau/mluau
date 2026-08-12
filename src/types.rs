use std::os::raw::{c_int, c_void};

use crate::error::Result;
use crate::state::{Lua, RawLua};

// Re-export mutex wrappers
pub use sync::XRc;
pub(crate) use sync::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XWeak};

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use either::Either;
pub(crate) use value_ref::ValueRef;

use std::collections::HashMap;

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

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

#[cfg(all(feature = "luau", feature = "send"))]
pub(crate) type NamecallCallback = XRc<dyn Fn(&RawLua, c_int) -> std::result::Result<c_int, crate::Value> + Send + 'static>;
#[cfg(all(feature = "luau", not(feature = "send")))]
pub(crate) type NamecallCallback = XRc<dyn Fn(&RawLua, c_int) -> std::result::Result<c_int, crate::Value> + 'static>;

#[cfg(all(feature = "luau", feature = "send"))]
pub(crate) type DynamicCallback = XRc<dyn Fn(&RawLua, &str, c_int) -> std::result::Result<c_int, crate::Value> + Send + 'static>;
#[cfg(all(feature = "luau", not(feature = "send")))]
pub(crate) type DynamicCallback = XRc<dyn Fn(&RawLua, &str, c_int) -> std::result::Result<c_int, crate::Value> + 'static>;

pub struct NamecallMap {
    pub(crate) map: HashMap<String, NamecallCallback>,
    pub(crate) dynamic: Option<DynamicCallback>,
}

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

pub(crate) struct DestructedUserdata;

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
