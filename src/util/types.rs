use std::any::Any;
use std::os::raw::c_void;

use crate::types::{Callback, CallbackUpvalue};

#[cfg(all(not(feature = "lua51"), not(feature = "luajit")))]
use crate::types::ContinuationUpvalue;

#[cfg(feature = "luau")]
use crate::types::{NamecallCallbackUpvalue, NamecallMapUpvalue};

pub(crate) trait TypeKey: Any {
    fn type_key() -> *const c_void;
}

impl TypeKey for String {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static STRING_TYPE_KEY: u8 = 0;
        &STRING_TYPE_KEY as *const u8 as *const c_void
    }
}

impl TypeKey for Callback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static CALLBACK_TYPE_KEY: u8 = 0;
        &CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}

impl TypeKey for CallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;
        &CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(all(not(feature = "lua51"), not(feature = "luajit")))]
impl TypeKey for ContinuationUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static CONTINUATION_UPVALUE_TYPE_KEY: u8 = 0;
        &CONTINUATION_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "luau")]
impl TypeKey for NamecallCallbackUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static NAMECALL_CALLBACK_UPVALUE_TYPE_KEY: u8 = 0;
        &NAMECALL_CALLBACK_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(feature = "luau")]
impl TypeKey for NamecallMapUpvalue {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static NAMECALL_MAP_UPVALUE_TYPE_KEY: u8 = 0;
        &NAMECALL_MAP_UPVALUE_TYPE_KEY as *const u8 as *const c_void
    }
}

#[cfg(not(feature = "luau"))]
impl TypeKey for crate::types::HookCallback {
    #[inline(always)]
    fn type_key() -> *const c_void {
        static HOOK_CALLBACK_TYPE_KEY: u8 = 0;
        &HOOK_CALLBACK_TYPE_KEY as *const u8 as *const c_void
    }
}
