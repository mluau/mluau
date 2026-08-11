#[cfg(feature = "dynamic-userdata")]
use std::any::Any;
use std::os::raw::{c_int, c_void};
use std::{mem, ptr};

use crate::error::Result;
use crate::userdata::collect_userdata;

#[cfg(feature = "dynamic-userdata")]
use crate::userdata::collect_userdata_dyn;
#[cfg(feature = "dynamic-userdata")]
use crate::userdata::DynamicUserDataPtr;

// Internally uses 3 stack spaces, does not call checkstack.
#[inline]
pub(crate) unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T) -> Result<*mut T> {
    let size = const { mem::size_of::<T>() };

    let ud_ptr = protect_lua!(state, 0, 1, |state| {
        ffi::lua_newuserdatadtor(state, size, collect_userdata::<T>)
    })? as *mut T;

    ptr::write(ud_ptr, t);
    Ok(ud_ptr)
}

#[inline]
pub(crate) unsafe fn push_fat_cclosure<T>(
    state: *mut ffi::lua_State,
    func: T,
    call_callback: ffi::lua_CFunction,
    debugname: *const std::os::raw::c_char,
    cont_callback: Option<ffi::lua_Continuation>,
) -> Result<()> {
    unsafe extern "C" fn dtor<T>(
        state: *mut ffi::lua_State,
        data: *mut c_void,
        _sz: usize,
    ) {
        // Almost none Lua operations are allowed when destructor is running,
        // so we need to set a flag to prevent calling any Lua functions
        let extra = (*ffi::lua_callbacks(state)).userdata as *mut crate::state::ExtraData;
        (*extra).running_gc = true;
        // Luau does not support _any_ panics in destructors (they are declared as "C", NOT as "C-unwind"),
        // so any panics will trigger `abort()`.
        ptr::drop_in_place(data as *mut T);
        (*extra).running_gc = false;
    }

    let ptr = protect_lua!(state, 0, 1, |state| {
        ffi::lua_pushcclosurewithdatak(
            state,
            call_callback,
            debugname,
            cont_callback,
            mem::size_of::<T>(),
            Some(dtor::<T>),
        )
    })? as *mut T;
    
    ptr::write(ptr, func);
    Ok(())
}

// Internally uses 3 stack spaces, does not call checkstack.
//
// mt_ptr is a pointer to the metatable for this userdata
// which is needed during the destructor call to clear out
// the associated data of the dynamic userdata.
#[inline]
#[cfg(feature = "dynamic-userdata")]
pub(crate) unsafe fn push_userdata_dyn(
    state: *mut ffi::lua_State,
    data: Box<dyn Any + Send + Sync>,
) -> Result<*mut DynamicUserDataPtr> {
    let size = const { mem::size_of::<DynamicUserDataPtr>() };

    let ud_ptr = protect_lua!(state, 0, 1, |state| {
        ffi::lua_newuserdatadtor(state, size, collect_userdata_dyn)
    })? as *mut DynamicUserDataPtr;

    let t = DynamicUserDataPtr { data };
    ptr::write(ud_ptr, t);
    Ok(ud_ptr)
}

#[inline]
#[track_caller]
pub(crate) unsafe fn get_userdata<T>(state: *mut ffi::lua_State, index: c_int) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    mlua_debug_assert!(!ud.is_null(), "userdata pointer is null");
    ud
}

/// Unwraps `T` from the Lua userdata and invalidating it by setting the special "destructed"
/// metatable.
///
/// This method does not check that userdata is of type `T` and was not previously invalidated.
///
/// Uses 1 extra stack space, does not call checkstack.
pub(crate) unsafe fn take_userdata<T>(state: *mut ffi::lua_State, idx: c_int) -> T {
    #[rustfmt::skip]
    let idx = if idx < 0 { ffi::lua_absindex(state, idx) } else { idx };

    // Update the metatable of this userdata to a special one with no `__gc` method and with
    // metamethods that trigger an error on access.
    // We do this so that it will not be double dropped or used after being dropped.
    get_destructed_userdata_metatable(state);
    ffi::lua_setmetatable(state, idx);
    let ud = get_userdata::<T>(state, idx);

    // Update userdata tag to disable destructor and mark as destructed

    ffi::lua_setuserdatatag(state, idx, 1);

    ptr::read(ud)
}

pub(crate) unsafe fn get_destructed_userdata_metatable(state: *mut ffi::lua_State) {
    let key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

pub(crate) static DESTRUCTED_USERDATA_METATABLE: u8 = 0;
