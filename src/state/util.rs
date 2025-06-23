use crate::IntoLuaMulti;
use std::mem::take;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::state::extra::RefThread;
use crate::state::{ExtraData, RawLua};
use crate::util::{self, check_stack, get_internal_metatable, push_string, StackGuard, WrappedFailure};

#[cfg(all(not(feature = "lua51"), not(feature = "luajit"), not(feature = "luau")))]
use crate::{types::ContinuationUpvalue, util::get_userdata};

struct StateGuard<'a>(&'a RawLua, *mut ffi::lua_State);

impl<'a> StateGuard<'a> {
    fn new(inner: &'a RawLua, mut state: *mut ffi::lua_State) -> Self {
        state = inner.state.replace(state);
        Self(inner, state)
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        self.0.state.set(self.1);
    }
}

pub(crate) enum PreallocatedFailure {
    New(*mut WrappedFailure),
    Reserved,
}

impl PreallocatedFailure {
    unsafe fn reserve(state: *mut ffi::lua_State, extra: *mut ExtraData) -> Self {
        if (*extra).wrapped_failure_top > 0 {
            (*extra).wrapped_failure_top -= 1;
            return PreallocatedFailure::Reserved;
        }

        // We need to check stack for Luau in case when callback is called from interrupt
        // See https://github.com/luau-lang/luau/issues/446 and mlua #142 and #153
        #[cfg(feature = "luau")]
        ffi::lua_rawcheckstack(state, 2);
        // Place it to the beginning of the stack
        let ud = WrappedFailure::new_userdata(state);
        ffi::lua_insert(state, 1);
        PreallocatedFailure::New(ud)
    }

    #[cold]
    unsafe fn r#use(&self, state: *mut ffi::lua_State, extra: *mut ExtraData) -> *mut WrappedFailure {
        let ref_thread = &(*extra).ref_thread_internal;
        match *self {
            PreallocatedFailure::New(ud) => {
                ffi::lua_settop(state, 1);
                ud
            }
            PreallocatedFailure::Reserved => {
                let index = (*extra).wrapped_failure_pool.pop().unwrap();
                ffi::lua_settop(state, 0);
                #[cfg(feature = "luau")]
                ffi::lua_rawcheckstack(state, 2);
                ffi::lua_xpush(ref_thread.ref_thread, state, index);
                ffi::lua_pushnil(ref_thread.ref_thread);
                ffi::lua_replace(ref_thread.ref_thread, index);
                (*extra).ref_thread_internal.free.push(index);
                ffi::lua_touserdata(state, -1) as *mut WrappedFailure
            }
        }
    }

    unsafe fn release(self, state: *mut ffi::lua_State, extra: *mut ExtraData) {
        let ref_thread = &(*extra).ref_thread_internal;
        match self {
            PreallocatedFailure::New(_) => {
                ffi::lua_rotate(state, 1, -1);
                ffi::lua_xmove(state, ref_thread.ref_thread, 1);
                let index = ref_stack_pop_internal(extra);
                (*extra).wrapped_failure_pool.push(index);
                (*extra).wrapped_failure_top += 1;
            }
            PreallocatedFailure::Reserved => (*extra).wrapped_failure_top += 1,
        }
    }
}

unsafe fn push_error_string(state: *mut ffi::lua_State, extra: *mut ExtraData, s: impl AsRef<[u8]>) {
    unsafe fn push_error_string_errorable(
        state: *mut ffi::lua_State,
        extra: *mut ExtraData,
        s: impl AsRef<[u8]>,
    ) -> Result<()> {
        let rawlua = (*extra).raw_lua();
        if rawlua.unlikely_memory_error() {
            push_string(state, s.as_ref(), false)?;
            return Ok(());
        }

        let _sg = StackGuard::new(state);
        check_stack(state, 3)?;
        push_string(state, s.as_ref(), true)?;
        Ok(())
    }

    if push_error_string_errorable(state, extra, s).is_err() {
        // If we cannot push the error string, we need to fallback to error userdata
        let s = "memory error".to_string();
        ffi::lua_pushlstring(state, s.as_ptr() as *const _, s.len());
        drop(s); // Lua copies the string, so we can drop it now
        ffi::lua_error(state);
    }
}

// An optimized version of `callback_error` that does not allocate `WrappedFailure` userdata
// and instead reuses unused values from previous calls (or allocates new).
pub(crate) unsafe fn callback_error_ext<F, R>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    wrap_error: bool,
    f: F,
) -> R
where
    F: FnOnce(*mut ExtraData, c_int) -> Result<R>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            // Return unused `WrappedFailure` to the pool
            prealloc_failure.release(state, extra);
            r
        }
        Ok(Err(err)) => {
            if (*extra).disable_error_userdata {
                push_error_string(state, extra, err.to_string());
            }

            let wrapped_error = prealloc_failure.r#use(state, extra);

            if !wrap_error {
                ptr::write(wrapped_error, WrappedFailure::Error(err));
                get_internal_metatable::<WrappedFailure>(state);
                ffi::lua_setmetatable(state, -2);
                ffi::lua_error(state)
            }

            // Build `CallbackError` with traceback
            let traceback = if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
                ffi::luaL_traceback(state, state, ptr::null(), 0);
                let traceback = util::to_string(state, -1);
                ffi::lua_pop(state, 1);
                traceback
            } else {
                "<not enough stack space for traceback>".to_string()
            };
            let cause = Arc::new(err);
            ptr::write(
                wrapped_error,
                WrappedFailure::Error(Error::CallbackError { traceback, cause }),
            );
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);

            ffi::lua_error(state)
        }
        Err(p) => {
            if (*extra).disable_error_userdata {
                // Push the error message directly onto the stack
                let err_msg = {
                    // If downcastable to String, use it
                    if let Some(s) = p.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = p.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        // Otherwise, use the debug representation
                        format!("Panic occurred in callback: {:?}", p)
                    }
                };

                push_error_string(state, extra, err_msg);
            }

            let wrapped_panic = prealloc_failure.r#use(state, extra);
            ptr::write(wrapped_panic, WrappedFailure::Panic(Some(p)));
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_error(state)
        }
    }
}

/// An yieldable version of `callback_error_ext`
///
/// Unlike ``callback_error_ext``, this method requires a c_int return
/// and not a generic R
pub(crate) unsafe fn callback_error_ext_yieldable<F>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    wrap_error: bool,
    f: F,
    #[allow(unused_variables)] in_callback_with_continuation: bool,
) -> c_int
where
    F: FnOnce(*mut ExtraData, c_int) -> Result<c_int>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    // We cannot shadow Rust errors with Lua ones, so we need to reserve pre-allocated memory
    // to store a wrapped failure (error or panic) *before* we proceed.
    let prealloc_failure = PreallocatedFailure::reserve(state, extra);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            // Return unused `WrappedFailure` to the pool
            //
            // In either case, we cannot use it in the yield case anyways due to the lua_pop call
            // so drop it properly now while we can.
            prealloc_failure.release(state, extra);

            let raw = extra.as_ref().unwrap_unchecked().raw_lua();
            let values = take(&mut extra.as_mut().unwrap_unchecked().yielded_values);

            if let Some(values) = values {
                // A note on Luau
                //
                // When using the yieldable continuations fflag (and in future when the fflag gets removed and
                // yieldable continuations) becomes default, we must either pop the top of the
                // stack on the state we are resuming or somehow store the number of
                // args on top of stack pre-yield and then subtract in the resume in order to get predictable
                // behaviour here. See https://github.com/luau-lang/luau/issues/1867 for more information
                //
                // In this case, popping is easier and leads to less bugs/more ergonomic API.

                // We need to pop/clear stack early, then push args
                ffi::lua_pop(state, -1);

                match values.push_into_specified_stack_multi(raw, state) {
                    Ok(nargs) => {
                        #[cfg(all(not(feature = "luau"), not(feature = "lua51"), not(feature = "luajit")))]
                        {
                            // Yield to a continuation. Unlike luau, we need to do this manually and on the
                            // fly using a yieldk call
                            if in_callback_with_continuation {
                                // On Lua 5.2, status and ctx are not present, so use 0 as status for
                                // compatibility
                                #[cfg(feature = "lua52")]
                                unsafe extern "C-unwind" fn cont_callback(
                                    state: *mut ffi::lua_State,
                                ) -> c_int {
                                    let upvalue =
                                        get_userdata::<ContinuationUpvalue>(state, ffi::lua_upvalueindex(1));
                                    callback_error_ext_yieldable(
                                        state,
                                        (*upvalue).extra.get(),
                                        true,
                                        |extra, nargs| {
                                            // Lua ensures that `LUA_MINSTACK` stack spaces are available
                                            // (after pushing arguments)
                                            // The lock must be already held as the callback is executed
                                            let rawlua = (*extra).raw_lua();
                                            match (*upvalue).data {
                                                Some(ref func) => (func.1)(rawlua, nargs, 0),
                                                None => Err(Error::CallbackDestructed),
                                            }
                                        },
                                        true,
                                    )
                                }

                                // Lua 5.3/5.4 case
                                #[cfg(not(feature = "lua52"))]
                                unsafe extern "C-unwind" fn cont_callback(
                                    state: *mut ffi::lua_State,
                                    status: c_int,
                                    _ctx: ffi::lua_KContext,
                                ) -> c_int {
                                    let upvalue =
                                        get_userdata::<ContinuationUpvalue>(state, ffi::lua_upvalueindex(1));
                                    callback_error_ext_yieldable(
                                        state,
                                        (*upvalue).extra.get(),
                                        true,
                                        |extra, nargs| {
                                            // Lua ensures that `LUA_MINSTACK` stack spaces are available
                                            // (after pushing arguments)
                                            // The lock must be already held as the callback is executed
                                            let rawlua = (*extra).raw_lua();
                                            match (*upvalue).data {
                                                Some(ref func) => (func.1)(rawlua, nargs, status),
                                                None => Err(Error::CallbackDestructed),
                                            }
                                        },
                                        true,
                                    )
                                }

                                return ffi::lua_yieldc(state, nargs, cont_callback);
                            }
                        }

                        return ffi::lua_yield(state, nargs);
                    }
                    Err(err) => {
                        if (*extra).disable_error_userdata {
                            push_error_string(state, extra, err.to_string());
                        }

                        // Make a *new* preallocated failure, and then do normal wrap_error
                        let prealloc_failure = PreallocatedFailure::reserve(state, extra);
                        let wrapped_panic = prealloc_failure.r#use(state, extra);
                        ptr::write(wrapped_panic, WrappedFailure::Error(err));
                        get_internal_metatable::<WrappedFailure>(state);
                        ffi::lua_setmetatable(state, -2);
                        ffi::lua_error(state);
                    }
                }
            }

            r
        }
        Ok(Err(err)) => {
            if (*extra).disable_error_userdata {
                push_error_string(state, extra, err.to_string());
            }

            let wrapped_error = prealloc_failure.r#use(state, extra);

            if !wrap_error {
                ptr::write(wrapped_error, WrappedFailure::Error(err));
                get_internal_metatable::<WrappedFailure>(state);
                ffi::lua_setmetatable(state, -2);
                ffi::lua_error(state)
            }

            // Build `CallbackError` with traceback
            let traceback = if ffi::lua_checkstack(state, ffi::LUA_TRACEBACK_STACK) != 0 {
                ffi::luaL_traceback(state, state, ptr::null(), 0);
                let traceback = util::to_string(state, -1);
                ffi::lua_pop(state, 1);
                traceback
            } else {
                "<not enough stack space for traceback>".to_string()
            };
            let cause = Arc::new(err);
            ptr::write(
                wrapped_error,
                WrappedFailure::Error(Error::CallbackError { traceback, cause }),
            );
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);

            ffi::lua_error(state)
        }
        Err(p) => {
            if (*extra).disable_error_userdata {
                // Push the error message directly onto the stack
                let err_msg = {
                    // If downcastable to String, use it
                    if let Some(s) = p.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = p.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        // Otherwise, use the debug representation
                        format!("Panic occurred in callback: {:?}", p)
                    }
                };

                push_error_string(state, extra, err_msg);
            }

            let wrapped_panic = prealloc_failure.r#use(state, extra);
            ptr::write(wrapped_panic, WrappedFailure::Panic(Some(p)));
            get_internal_metatable::<WrappedFailure>(state);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_error(state)
        }
    }
}

pub(super) unsafe fn ref_stack_pop_internal(extra: *mut ExtraData) -> c_int {
    let extra = &mut *extra;
    let ref_th = &mut extra.ref_thread_internal;

    if let Some(free) = ref_th.free.pop() {
        ffi::lua_replace(ref_th.ref_thread, free);
        return free;
    }

    // Try to grow max stack size
    if ref_th.stack_top >= ref_th.stack_size {
        let mut inc = ref_th.stack_size; // Try to double stack size
        while inc > 0 && ffi::lua_checkstack(ref_th.ref_thread, inc) == 0 {
            inc /= 2;
        }
        if inc == 0 {
            // Pop item on top of the stack to avoid stack leaking and successfully run destructors
            // during unwinding.
            ffi::lua_pop(ref_th.ref_thread, 1);
            let top = ref_th.stack_top;
            // It is a user error to create enough references to exhaust the Lua max stack size for
            // the ref thread. This should never happen for the internal aux thread but still
            panic!("internal error: cannot create a Lua reference, out of internal auxiliary stack space (used {top} slots)");
        }
        ref_th.stack_size += inc;
    }
    ref_th.stack_top += 1;
    return ref_th.stack_top;
}

// Run a comparison function on two Lua references from different auxiliary threads.
pub(crate) unsafe fn compare_refs<R>(
    extra: *mut ExtraData,
    aux_thread_a: usize,
    aux_thread_a_index: c_int,
    aux_thread_b: usize,
    aux_thread_b_index: c_int,
    f: impl FnOnce(*mut ffi::lua_State, c_int, c_int) -> R,
) -> R {
    let extra = &mut *extra;

    if aux_thread_a == aux_thread_b {
        // If both threads are the same, just return the value at the index
        let th = &mut extra.ref_thread[aux_thread_a];
        return f(th.ref_thread, aux_thread_a_index, aux_thread_b_index);
    }

    let th_a = &extra.ref_thread[aux_thread_a];
    let th_b = &extra.ref_thread[aux_thread_b];
    let internal_thread = &mut extra.ref_thread_internal;

    // 4 spaces needed: idx element on A, idx element on B
    check_stack(internal_thread.ref_thread, 2)
        .expect("internal error: cannot merge references, out of internal auxiliary stack space");

    // Push the index element from thread A to top
    ffi::lua_xpush(th_a.ref_thread, internal_thread.ref_thread, aux_thread_a_index);
    // Push the index element from thread B to top
    ffi::lua_xpush(th_b.ref_thread, internal_thread.ref_thread, aux_thread_b_index);
    // Now we have the following stack:
    // - index element from thread A (1) [copy from pushvalue]
    // - index element from thread B (2) [copy from pushvalue]
    // We want to compare the index elements from both threads, so use 3 and 4 as indices
    let result = f(internal_thread.ref_thread, -1, -2);

    // Pop the top 2 elements to clean the copies
    ffi::lua_pop(internal_thread.ref_thread, 2);

    result
}

pub(crate) unsafe fn get_next_spot(extra: *mut ExtraData) -> (usize, c_int, bool) {
    if extra.is_null() {
        panic!("get_next_spot called with null extra pointer");
    }
    let extra = &mut *extra;

    // Find the first thread with a free slot
    for (i, ref_th) in extra.ref_thread.iter_mut().enumerate() {
        if let Some(free) = ref_th.free.pop() {
            return (i, free, true);
        }

        // Try to grow max stack size
        if ref_th.stack_top >= ref_th.stack_size {
            let mut inc = ref_th.stack_size; // Try to double stack size
            while inc > 0 && ffi::lua_checkstack(ref_th.ref_thread, inc + 1) == 0 {
                inc /= 2;
            }
            if inc == 0 {
                continue; // No stack space available, try next thread
            }
            ref_th.stack_size += inc;
        }

        ref_th.stack_top += 1;
        return (i, ref_th.stack_top, false);
    }

    // No free slots found, create a new one
    let new_ref_thread = RefThread::new(extra.raw_lua().state());
    extra.ref_thread.push(new_ref_thread);
    return get_next_spot(extra);
}
