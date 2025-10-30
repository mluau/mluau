use std::cell::UnsafeCell;
use std::os::raw::{c_int, c_void};

#[cfg(not(feature = "luau"))]
use crate::debug::{Debug, HookTriggers};
use crate::error::Result;
use crate::state::{ExtraData, Lua, RawLua};

// Re-export mutex wrappers
pub(crate) use sync::{ArcReentrantMutexGuard, ReentrantMutex, ReentrantMutexGuard, XRc, XWeak};

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use either::Either;
pub use registry_key::RegistryKey;
pub(crate) use value_ref::ValueRef;

#[cfg(feature = "luau")]
use std::collections::HashMap;

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

/// A "light" userdata value. Equivalent to an unmanaged raw pointer.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct LightUserData(pub *mut c_void);

#[cfg(feature = "send")]
unsafe impl Send for LightUserData {}
#[cfg(feature = "send")]
unsafe impl Sync for LightUserData {}

#[cfg(feature = "send")]
pub(crate) type Callback = Box<dyn Fn(&RawLua, c_int) -> Result<c_int> + Send + 'static>;
#[cfg(not(feature = "send"))]
type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> Result<c_int> + 'a;

pub(crate) type Callback = Box<CallbackFn<'static>>;

#[cfg(all(feature = "send", not(feature = "lua51"), not(feature = "luajit")))]
pub(crate) type Continuation = Box<dyn Fn(&RawLua, c_int, c_int) -> Result<c_int> + Send + 'static>;
#[cfg(all(not(feature = "send"), not(feature = "lua51"), not(feature = "luajit")))]
pub(crate) type Continuation = Box<dyn Fn(&RawLua, c_int, c_int) -> Result<c_int> + 'static>;

#[cfg(all(feature = "luau", feature = "send"))]
pub(crate) type NamecallCallback = XRc<dyn Fn(&RawLua, c_int) -> Result<c_int> + Send + 'static>;
#[cfg(all(feature = "luau", not(feature = "send")))]
pub(crate) type NamecallCallback = XRc<dyn Fn(&RawLua, c_int) -> Result<c_int> + 'static>;

#[cfg(all(feature = "luau", feature = "send"))]
pub(crate) type DynamicCallback = XRc<dyn Fn(&RawLua, &str, c_int) -> Result<c_int> + Send + 'static>;
#[cfg(all(feature = "luau", not(feature = "send")))]
pub(crate) type DynamicCallback = XRc<dyn Fn(&RawLua, &str, c_int) -> Result<c_int> + 'static>;

pub(crate) struct Upvalue<T> {
    pub(crate) data: T,
    pub(crate) extra: XRc<UnsafeCell<ExtraData>>,
}

pub(crate) type CallbackUpvalue = Upvalue<Option<Callback>>;

#[cfg(all(not(feature = "lua51"), not(feature = "luajit")))]
pub(crate) type ContinuationUpvalue = Upvalue<Option<(Callback, Continuation)>>;
#[cfg(feature = "luau")]
pub(crate) type NamecallCallbackUpvalue = Upvalue<Option<NamecallCallback>>;

#[cfg(feature = "luau")]
pub struct NamecallMap {
    pub(crate) map: HashMap<String, NamecallCallback>,
    pub(crate) dynamic: Option<DynamicCallback>,
}

#[cfg(feature = "luau")]
pub(crate) type NamecallMapUpvalue = Upvalue<Option<NamecallMap>>;

/// Type to set next Lua VM action after executing interrupt or hook function.
pub enum VmState {
    Continue,
    /// Yield the current thread.
    ///
    /// Supported by Lua 5.3+ and Luau.
    Yield,
}

#[cfg(not(feature = "luau"))]
pub(crate) enum HookKind {
    Global,
    Thread(HookTriggers, HookCallback),
}

#[cfg(all(feature = "send", not(feature = "luau")))]
pub(crate) type HookCallback = XRc<dyn Fn(&Lua, &Debug) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), not(feature = "luau")))]
pub(crate) type HookCallback = XRc<dyn Fn(&Lua, &Debug) -> Result<VmState>>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type InterruptCallback = XRc<dyn Fn(&Lua) -> Result<VmState>>;

#[cfg(feature = "luau")]
pub(crate) type GcInterruptCallback = XRc<dyn Fn(&Lua, c_int) -> ()>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type ThreadCreationCallback = XRc<dyn Fn(&Lua, crate::Thread) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type ThreadCreationCallback = XRc<dyn Fn(&Lua, crate::Thread) -> Result<()>>;

#[cfg(all(feature = "send", feature = "luau"))]
pub(crate) type ThreadCollectionCallback = XRc<dyn Fn(crate::LightUserData) + Send>;

#[cfg(all(not(feature = "send"), feature = "luau"))]
pub(crate) type ThreadCollectionCallback = XRc<dyn Fn(crate::LightUserData)>;

#[cfg(all(feature = "send", feature = "lua54"))]
pub(crate) type WarnCallback = XRc<dyn Fn(&Lua, &str, bool) -> Result<()> + Send>;

#[cfg(all(not(feature = "send"), feature = "lua54"))]
pub(crate) type WarnCallback = XRc<dyn Fn(&Lua, &str, bool) -> Result<()>>;

/// A trait that adds `Send` requirement if `send` feature is enabled.
#[cfg(feature = "send")]
pub trait MaybeSend: Send {}
#[cfg(feature = "send")]
impl<T: Send> MaybeSend for T {}

#[cfg(not(feature = "send"))]
pub trait MaybeSend {}
#[cfg(not(feature = "send"))]
impl<T> MaybeSend for T {}

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
mod registry_key;
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
