macro_rules! bug_msg {
    ($arg:expr) => {
        concat!(
            "mlua internal error: ",
            $arg,
            " (this is a bug, please file an issue)"
        )
    };
}

macro_rules! cstr {
    ($s:expr) => {
        concat!($s, "\0") as *const str as *const [::std::os::raw::c_char] as *const ::std::os::raw::c_char
    };
}

macro_rules! mlua_panic {
    ($msg:expr) => {
        panic!(bug_msg!($msg))
    };

    ($msg:expr,) => {
        mlua_panic!($msg)
    };

    ($msg:expr, $($arg:expr),+) => {
        panic!(bug_msg!($msg), $($arg),+)
    };

    ($msg:expr, $($arg:expr),+,) => {
        mlua_panic!($msg, $($arg),+)
    };
}

macro_rules! mlua_assert {
    ($cond:expr, $msg:expr) => {
        assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        mlua_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        mlua_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! mlua_debug_assert {
    ($cond:expr, $msg:expr) => {
        debug_assert!($cond, bug_msg!($msg));
    };

    ($cond:expr, $msg:expr,) => {
        mlua_debug_assert!($cond, $msg);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+) => {
        debug_assert!($cond, bug_msg!($msg), $($arg),+);
    };

    ($cond:expr, $msg:expr, $($arg:expr),+,) => {
        mlua_debug_assert!($cond, $msg, $($arg),+);
    };
}

macro_rules! mlua_expect {
    ($res:expr, $msg:expr) => {
        $res.expect(bug_msg!($msg))
    };

    ($res:expr, $msg:expr,) => {
        mlua_expect!($res, $msg)
    };
}

#[cfg(feature = "module")]
#[doc(hidden)]
#[macro_export]
macro_rules! require_module_feature {
    () => {};
}

#[cfg(not(feature = "module"))]
#[doc(hidden)]
#[macro_export]
macro_rules! require_module_feature {
    () => {
        compile_error!("Feature `module` must be enabled in the `mlua` crate");
    };
}

macro_rules! protect_lua {
    ($state:expr, $nargs:expr, $nresults:expr, $f:expr) => {
        crate::util::protect_lua_closure($state, $nargs, $nresults, $f)
    };

    ($state:expr, $nargs:expr, $nresults:expr, fn($state_inner:ident) $code:expr) => {{
        use ::std::os::raw::c_int;
        unsafe extern "C-unwind" fn do_call($state_inner: *mut ffi::lua_State) -> c_int {
            $code;
            let nresults = $nresults;
            if nresults == ::ffi::LUA_MULTRET {
                ffi::lua_gettop($state_inner)
            } else {
                nresults
            }
        }

        crate::util::protect_lua_call($state, $nargs, do_call)
    }};
}

/// Fast-protect uses luau_try on Luau to call a function and catch panics/exceptions.
#[cfg(feature = "luau")]
macro_rules! fast_protect {
    ($state:expr, fn($state_inner:ident) $code:expr) => {
        // Make use of luau_try for performance
        {
            unsafe extern "C-unwind" fn do_call($state_inner: *mut ffi::lua_State, _data: *mut ::std::ffi::c_void) -> *mut ::std::ffi::c_void {
                let ret = $code;
                ret as *mut ::std::ffi::c_void
            }

            let ret = crate::ffi::luau_try($state, do_call, ::std::ptr::null_mut());
            match ret.status {
                0 => Ok(ret.ret),
                1 => {
                    // Pop error message from stack
                    use crate::util::to_string;
                    let s = to_string($state, -1);
                    ffi::lua_pop($state, 1);
                    Err(Error::RuntimeError(s))
                }
                _ => Err(Error::RuntimeError("Unknown error in luau_try".to_string())),
            }
        }
    }
}