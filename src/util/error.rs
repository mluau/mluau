use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::ptr;

use crate::error::{Error, Result};
use crate::memory::MemoryState;
use crate::state::{ExtraData, callback_error_ext};
use crate::util::{
    check_stack, push_table, rawset_field, to_string, DESTRUCTED_USERDATA_METATABLE,
};

// Alias to `callback_error_ext`
unsafe fn callback_error<F, R>(state: *mut ffi::lua_State, f: F) -> R
where
    F: FnOnce(c_int) -> Result<R>,
{
    let extra = ExtraData::get(state);
    callback_error_ext(state, extra, |extra, status| f(status).map_err(|e| crate::state::util::map_err_to_value((*extra).raw_lua().lua(), e)))
}

// Pops an error off of the stack and returns it. The specific behavior depends on the type of the
// error at the top of the stack:
//   1) If the error is actually a panic, this will continue the panic.
//   2) If the error on the top of the stack is actually an error, just returns it.
//   3) Otherwise, interprets the error as the appropriate lua error.
// Uses 2 stack spaces, does not call checkstack.
pub(crate) unsafe fn pop_error(state: *mut ffi::lua_State, err_code: c_int) -> Error {
    mlua_debug_assert!(
        err_code != ffi::LUA_OK && err_code != ffi::LUA_YIELD,
        "pop_error called with non-error return code"
    );


    let err_string = to_string(state, -1);
    ffi::lua_pop(state, 1);

    match err_code {
        ffi::LUA_ERRRUN => Error::RuntimeError(err_string),
        ffi::LUA_ERRSYNTAX => {
            Error::SyntaxError {
                // This seems terrible, but as far as I can tell, this is exactly what the
                // stock Lua REPL does.
                incomplete_input: err_string.ends_with("<eof>") || err_string.ends_with("'<eof>'"),
                message: err_string,
            }
        }
        ffi::LUA_ERRERR => {
            // This error is raised when the error handler raises an error too many times
            // recursively, and continuing to trigger the error handler would cause a stack
            // overflow. It is not very useful to differentiate between this and "ordinary"
            // runtime errors, so we handle them the same way.
            Error::RuntimeError(err_string)
        }
        ffi::LUA_ERRMEM => Error::MemoryError(err_string),
        #[cfg(any(feature = "lua53", feature = "lua52"))]
        ffi::LUA_ERRGCMM => Error::GarbageCollectorError(err_string),
        _ => mlua_panic!("unrecognized lua error code"),
    }
}

unsafe fn push_cached_cfunction(
    state: *mut ffi::lua_State,
    f: unsafe extern "C-unwind" fn(*mut ffi::lua_State) -> c_int,
) {
    ffi::lua_pushlightuserdata(state, f as usize as *mut c_void);
    ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);
    if ffi::lua_type(state, -1) == ffi::LUA_TNIL {
        ffi::lua_pop(state, 1);
        ffi::lua_pushcfunction(state, f);
        ffi::lua_pushlightuserdata(state, f as usize as *mut c_void);
        ffi::lua_pushvalue(state, -2);
        ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);
    }
}

unsafe fn push_error_traceback(state: *mut ffi::lua_State) {
    use crate::state::ExtraData;
    let extra = ExtraData::get(state);
    if !extra.is_null() {
        ffi::lua_xpush(
            (*extra).ref_thread_internal.ref_thread,
            state,
            ExtraData::ERROR_TRACEBACK_IDX,
        );
    } else {
        push_cached_cfunction(state, error_traceback);
    }
}

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` is the same as the the parameter to `lua_pcall`, and `nresults` is
// always `LUA_MULTRET`. Provided function must *not* panic, and since it will generally be
// longjmping, should not contain any values that implements Drop.
// Internally uses 2 extra stack spaces, and does not call checkstack.
pub(crate) unsafe fn protect_lua_call(
    state: *mut ffi::lua_State,
    nargs: c_int,
    f: unsafe extern "C-unwind" fn(*mut ffi::lua_State) -> c_int,
) -> Result<()> {
    let stack_start = ffi::lua_gettop(state) - nargs;

    MemoryState::relax_limit_with(state, || {
        push_error_traceback(state);
        push_cached_cfunction(state, f);
    });
    if nargs > 0 {
        ffi::lua_rotate(state, stack_start + 1, 2);
    }

    let ret = ffi::lua_pcall(state, nargs, ffi::LUA_MULTRET, stack_start + 1);
    ffi::lua_remove(state, stack_start + 1);

    if ret == ffi::LUA_OK {
        Ok(())
    } else {
        Err(pop_error(state, ret))
    }
}

// Call a function that calls into the Lua API and may trigger a Lua error (longjmp) in a safe way.
// Wraps the inner function in a call to `lua_pcall`, so the inner function only has access to a
// limited lua stack. `nargs` and `nresults` are similar to the parameters of `lua_pcall`, but the
// given function return type is not the return value count, instead the inner function return
// values are assumed to match the `nresults` param. Provided function must *not* panic, and since
// it will generally be longjmping, should not contain any values that implements Drop.
// Internally uses 3 extra stack spaces, and does not call checkstack.
pub(crate) unsafe fn protect_lua_closure<F, R>(
    state: *mut ffi::lua_State,
    nargs: c_int,
    nresults: c_int,
    f: F,
) -> Result<R>
where
    F: FnOnce(*mut ffi::lua_State) -> R,
    R: Copy,
{
    struct Params<F, R> {
        function: Option<F>,
        result: MaybeUninit<R>,
        nresults: c_int,
    }

    unsafe extern "C-unwind" fn do_call<F, R>(state: *mut ffi::lua_State) -> c_int
    where
        F: FnOnce(*mut ffi::lua_State) -> R,
        R: Copy,
    {
        let params = ffi::lua_tolightuserdata(state, -1) as *mut Params<F, R>;
        ffi::lua_pop(state, 1);

        let f = (*params).function.take().unwrap();
        (*params).result.write(f(state));

        if (*params).nresults == ffi::LUA_MULTRET {
            ffi::lua_gettop(state)
        } else {
            (*params).nresults
        }
    }

    let stack_start = ffi::lua_gettop(state) - nargs;

    MemoryState::relax_limit_with(state, || {
        push_error_traceback(state);
        push_cached_cfunction(state, do_call::<F, R>);
    });
    if nargs > 0 {
        ffi::lua_rotate(state, stack_start + 1, 2);
    }

    let mut params = Params {
        function: Some(f),
        result: MaybeUninit::uninit(),
        nresults,
    };

    ffi::lua_pushlightuserdata(state, &mut params as *mut Params<F, R> as *mut c_void);
    let ret = ffi::lua_pcall(state, nargs + 1, nresults, stack_start + 1);
    ffi::lua_remove(state, stack_start + 1); // remove error handler

    if ret == ffi::LUA_OK {
        // `LUA_OK` is only returned when the `do_call` function has completed successfully, so
        // `params.result` is definitely initialized.
        Ok(params.result.assume_init())
    } else {
        Err(pop_error(state, ret))
    }
}

pub(crate) unsafe extern "C-unwind" fn error_traceback(state: *mut ffi::lua_State) -> c_int {
    // Luau calls error handler for memory allocation errors, skip it
    // See https://github.com/luau-lang/luau/issues/880

    if MemoryState::limit_reached(state) {
        return 0;
    }

    if ffi::lua_checkstack(state, 2) == 0 {
        // If we don't have enough stack space to even check the error type, do
        // nothing so we don't risk shadowing a rust panic.
        return 1;
    }


    let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
    if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
        ffi::luaL_traceback(state, state, s, 0);
        ffi::lua_remove(state, -2);
    }

    1
}

// A variant of `error_traceback` that can safely inspect another (yielded) thread stack
pub(crate) unsafe fn error_traceback_thread(state: *mut ffi::lua_State, thread: *mut ffi::lua_State) {
    // Move error object to the main thread to safely call `__tostring` metamethod if present
    ffi::lua_xmove(thread, state, 1);

    let s = ffi::luaL_tolstring(state, -1, ptr::null_mut());
    if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
        ffi::luaL_traceback(state, thread, s, 0);
        ffi::lua_remove(state, -2);
    }
}

// Initialize the destructed userdata metatables.
pub(crate) unsafe fn init_destructed_userdata_registry(state: *mut ffi::lua_State) -> Result<()> {
    check_stack(state, 3)?;

    // Create destructed userdata metatable
    unsafe extern "C-unwind" fn destructed_error(state: *mut ffi::lua_State) -> c_int {
        callback_error(state, |_| Err(Error::UserDataDestructed))
    }

    push_table(state, 0, 2)?;
    ffi::lua_pushcfunction(state, destructed_error);
    for &method in &["__index", "__newindex"] {
        ffi::lua_pushvalue(state, -1);
        rawset_field(state, -3, method)?;
    }
    ffi::lua_pop(state, 1);

    protect_lua!(state, 1, 0, fn(state) {
        let destructed_mt_key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, destructed_mt_key);
    })?;

    Ok(())
}
