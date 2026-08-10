use crate::IntoLuaMulti;
use std::mem::take;
use std::os::raw::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::state::extra::{RefThread, REF_STACK_RESERVE};
use crate::state::{ExtraData, RawLua};
use crate::util::check_stack;

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

pub(crate) unsafe fn push_error_value(state: *mut ffi::lua_State, extra: *mut ExtraData, err: crate::Value) {
    let raw_lua = (*extra).raw_lua();
    let res = protect_lua!(state, 0, 1, |state| {
        let _ = check_stack(state, 1);
        crate::memory::MemoryState::relax_limit_with(state, || {
            raw_lua.push_value_at(&err, state);
        });
    });

    if res.is_err() {
        // Fallback case: we have no space to even copy the traceback as a external string so we have to
        // push the fallback memory error
        let _ = check_stack(state, 1);
        let ref_thread = (*extra).ref_thread_internal.ref_thread;
        ffi::lua_pushvalue(ref_thread, ExtraData::MEMORY_ERROR_IDX);
        ffi::lua_xmove(ref_thread, state, 1);
    }
}

#[inline(always)]
pub(crate) fn map_err_to_value<E: crate::traits::IntoLuaErr>(lua: &crate::Lua, err: E) -> crate::Value {
    use crate::traits::IntoLuaErr;
    match err.into_lua_err(lua) {
        Ok(v) => v,
        Err(e) => {
            e.to_string().into_lua_err(lua).unwrap_or(crate::Value::Nil)
        }
    }
}

#[inline(always)]
pub(crate) fn extract_panic_str(p: Box<dyn std::any::Any + Send + 'static>) -> String {
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

    // WARNING: It is a logic error for the payload in p to itself panic
    std::mem::forget(catch_unwind(AssertUnwindSafe(move || drop(p))));

    err_msg
}

// An optimized version of `callback_error` that does not allocate `WrappedFailure` userdata
// and instead reuses unused values from previous calls (or allocates new).
pub(crate) unsafe fn callback_error_ext<F, R>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    f: F,
) -> R
where
    F: FnOnce(*mut ExtraData, c_int) -> std::result::Result<R, crate::Value>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            r
        }
        Ok(Err(err)) => {
            push_error_value(state, extra, err);
            ffi::lua_error(state);
        }
        Err(p) => {
            // Push the error message directly onto the stack
            let err_msg = extract_panic_str(p);
            push_error_value(state, extra, map_err_to_value((*extra).raw_lua().lua(), err_msg));
            ffi::lua_error(state);
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
    f: F,
    #[allow(unused_variables)] in_callback_with_continuation: bool,
) -> c_int
where
    F: FnOnce(*mut ExtraData, c_int) -> std::result::Result<c_int, crate::Value>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    let nargs = ffi::lua_gettop(state);

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra, nargs)
    })) {
        Ok(Ok(r)) => {
            let raw = extra.as_ref().unwrap_unchecked().raw_lua();

            {
                let values = take(&mut extra.as_mut().unwrap_unchecked().yielded_values);

                if let Some(values) = values {
                    // A note on Luau
                    //
                    // When using the yieldable continuations fflag (and in future when the fflag gets removed
                    // and yieldable continuations) becomes default, we must either pop
                    // the top of the stack on the state we are resuming or somehow store
                    // the number of args on top of stack pre-yield and then subtract in
                    // the resume in order to get predictable behaviour here. See https://github.com/luau-lang/luau/issues/1867 for more information
                    //
                    // In this case, popping is easier and leads to less bugs/more ergonomic API.

                    // We need to pop/clear stack early, then push args
                    ffi::lua_pop(state, -1);

                    match values.push_into_specified_stack_multi(raw, state) {
                        Ok(nargs) => {
                            return ffi::lua_yield(state, nargs);
                        }
                        Err(err) => {
                            push_error_value(state, extra, map_err_to_value(raw.lua(), err));
                            ffi::lua_error(state);
                        }
                    }
                }
            }

            r
        }
        Ok(Err(err)) => {
            push_error_value(state, extra, err);
            ffi::lua_error(state);
        }
        Err(p) => {
            // Push the error message directly onto the stack
            let err_msg = extract_panic_str(p);
            push_error_value(state, extra, map_err_to_value((*extra).raw_lua().lua(), err_msg));
            ffi::lua_error(state);
        }
    }
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

    // 2 spaces needed: idx element on A, idx element on B
    check_stack(internal_thread.ref_thread, 2)
        .expect("internal error: cannot merge references, out of internal auxiliary stack space");

    // Push the index element from thread A to top
    ffi::lua_xpush(th_a.ref_thread, internal_thread.ref_thread, aux_thread_a_index);
    // Push the index element from thread B to top
    ffi::lua_xpush(th_b.ref_thread, internal_thread.ref_thread, aux_thread_b_index);
    // Now we have the following stack:
    // - index element from thread A (2) [copy from xpush]
    // - index element from thread B (1) [copy from xpush]
    // We want to compare the index elements from both threads, so use -1 and -2 as indices
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
            while inc > 0 && ffi::lua_checkstack(ref_th.ref_thread, inc + REF_STACK_RESERVE) == 0 {
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
