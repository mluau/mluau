use super::lua::lua_State;
use core::ffi::c_int;

extern "C" {
    //pub fn lutec_opencrypto(state: *mut lua_State);
    pub fn lutec_openfs(state: *mut lua_State);
    pub fn lutec_openluau(state: *mut lua_State);
    //pub fn lutec_opennet(state: *mut lua_State);
    pub fn lutec_openprocess(state: *mut lua_State);
    pub fn lutec_opentask(state: *mut lua_State);
    pub fn lutec_openvm(state: *mut lua_State);
    pub fn lutec_opensystem(state: *mut lua_State);
    pub fn lutec_opentime(state: *mut lua_State) -> c_int;
    pub fn lutec_isruntimeloaded(state: *mut lua_State) -> c_int;
    pub fn lutec_setup_runtime(state: *mut lua_State);
    pub fn lutec_destroy_runtime(state: *mut lua_State) -> c_int;
    pub fn lutec_set_runtimeinitter(callback: lutec_setupState_init) -> c_int;
}

extern "C-unwind" {
    pub fn lutec_run_once(state: *mut lua_State) -> c_int;
}

#[repr(C)]
pub struct lua_State_wrapper {
    pub L: *mut lua_State,
}

#[allow(non_camel_case_types)]
#[repr(C)]
pub struct lutec_setupState {
    pub setup_lua_state: unsafe extern "C" fn(wrapper: *mut lua_State_wrapper),
}

// Populates function pointers in the given lutec_setupState.
pub type lutec_setupState_init = unsafe extern "C" fn(config: *mut lutec_setupState);
