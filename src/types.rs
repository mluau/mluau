use std::os::raw::{c_int, c_void};
use std::ptr::NonNull;

use crate::error::Result;
use crate::state::{Lua, RawLua};
use std::ops::Deref;
use std::rc::Rc as XRc;

pub use app_data::{AppData, AppDataRef, AppDataRefMut};
pub use either::Either;
pub(crate) use value_ref::ValueRef;

/// Type of Lua integer numbers.
pub type Integer = ffi::lua_Integer;
/// Type of Lua floating point numbers.
pub type Number = ffi::lua_Number;

/// A Luau-backed reference to a value of type `T` pinning both the Luau VM and the backer it came from
pub struct TypedRef<T: 'static, Backer: 'static + Clone, const TAG: c_int> {
    pub(crate) ud: Backer,
    // cached data ptr
    pub(crate) ptr: NonNull<T>,
    pub(crate) lua: XRc<RawLua>, // hold a strong ref to VM
}

impl<T: 'static, Backer: 'static + Clone, const TAG: c_int> Clone for TypedRef<T, Backer, TAG> {
    fn clone(&self) -> Self {
        Self {
            // new valueref refcount
            ud: self.ud.clone(), 
            ptr: self.ptr, // we can keep same ptr   
            lua: self.lua.clone(), // one new vm ref
        }
    }
}

impl<T: 'static, Backer: 'static + Clone + PartialEq, const TAG: c_int> PartialEq for TypedRef<T, Backer, TAG> {
    fn eq(&self, other: &Self) -> bool {
        if self.ud != other.ud {
            return false;
        }

        self.ptr == other.ptr
    }
}

impl<T: 'static + std::fmt::Debug, Backer: 'static + Clone, const TAG: c_int> std::fmt::Debug for TypedRef<T, Backer, TAG> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TypedRef")
            .field(&**self)
            .finish()
    }
}

impl<T: 'static, Backer: 'static + Clone, const TAG: c_int> Deref for TypedRef<T, Backer, TAG> {
    type Target = T;
    
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // SAFETY: ValueRef pins the TypedRef down w/ lua_refpool and _lua holds Lua VM alive
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: 'static, Backer: 'static + Clone, const TAG: c_int> TypedRef<T, Backer, TAG> {
    pub(crate) fn new(lua: XRc<RawLua>, data: NonNull<T>, ud: Backer) -> Self {
        Self { lua, ptr: data, ud }
    }

    pub(crate) fn new_opt(lua: XRc<RawLua>, data: Option<&T>, ud: Backer) -> Option<Self> {
        let ptr = data.map(|x| NonNull::from(x))?;
        Some(Self::new(lua, ptr, ud))
    }

    /// Returns a reference to the Lua reference backing `T`
    pub fn lua(&self) -> &Lua {
        &self.lua.lua()
    }

    /// Returns the backer that backs `T` consuming the TypedRef in the process
    pub fn into_backer(self) -> Backer {
        self.ud
    }

    /// Returns a reference to the backer that backs `T`
    pub fn backer(&self) -> &Backer {
        &self.ud
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

type CallbackFn<'a> = dyn Fn(&RawLua, c_int) -> std::result::Result<c_int, crate::Value> + 'a;

pub(crate) type Callback = Box<CallbackFn<'static>>;
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
mod value_ref;

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_any!(ValueRef: Send);
}
