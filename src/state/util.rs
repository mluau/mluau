use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::state::{ExtraData, RawLua};
use crate::string::vec_into_ext_parts_infailable;

pub(super) struct StateGuard<'a>(&'a RawLua, *mut ffi::lua_State);

impl<'a> StateGuard<'a> {
    pub(super) fn new(inner: &'a RawLua, mut state: *mut ffi::lua_State) -> Self {
        state = inner.state.replace(state);
        Self(inner, state)
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        self.0.state.set(self.1);
    }
}

/// Safety: This is only run in callbacks
/// 
/// Luau guarantees LUA_MINSTACK (20 slots) of stack space when entering a C callback.
pub(crate) unsafe fn push_callback_error(state: *mut ffi::lua_State, extra: *mut ExtraData, err: String) {
    let res = protect_lua!(state, 0, 1, |state| {
        use crate::string::ExternalString;
        let (ptr, len, userdata) = vec_into_ext_parts_infailable(err.into_bytes());
        let free_cb = Some(String::free_string as ffi::lua_StringFree);

        ffi::lua_pushexternalstring(
            state, ptr as *const _, len, userdata, free_cb
        );
    });

    if res.is_err() {
        // Fallback case: we have no space to even copy the traceback as a external string so we have to
        // push the fallback memory error
        let memory_error_ref = (*extra).memory_error_ref;
        ffi::lua_getrefpool(state, memory_error_ref);
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

// Deprecated
pub(crate) unsafe fn callback_error_ext<F, R>(
    state: *mut ffi::lua_State,
    mut extra: *mut ExtraData,
    f: F,
) -> R
where
    F: FnOnce(*mut ExtraData) -> crate::Result<R>,
{
    if extra.is_null() {
        extra = ExtraData::get(state);
    }

    match catch_unwind(AssertUnwindSafe(|| {
        let rawlua = (*extra).raw_lua();
        let _guard = StateGuard::new(rawlua, state);
        f(extra)
    })) {
        Ok(Ok(r)) => {
            r
        }
        Ok(Err(err)) => {
            push_callback_error(state, extra, err.to_string());
            ffi::lua_error(state);
        }
        Err(p) => {
            // Push the error message directly onto the stack
            let err_msg = extract_panic_str(p);
            push_callback_error(state, extra, err_msg);
            ffi::lua_error(state);
        }
    }
}