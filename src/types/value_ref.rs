use std::fmt;
use std::os::raw::{c_int, c_void};

use super::XRc;
use crate::state::{RawLua, WeakLua};

/// A reference to a Lua (complex) value stored in the Lua auxiliary thread.
#[derive(Clone)]
pub struct ValueRef {
    pub(crate) lua: WeakLua,
    pub(crate) ref_id: c_int,
    /// If `index_count` is `None`, the value does not need to be destroyed.
    pub(crate) index_count: Option<ValueRefIndex>,
}

/// A reference to a Lua value index in the auxiliary thread.
/// It's cheap to clone and can be used to track the number of references to a value.
#[derive(Clone)]
pub(crate) struct ValueRefIndex(pub(crate) XRc<c_int>);

impl From<c_int> for ValueRefIndex {
    #[inline]
    fn from(index: c_int) -> Self {
        ValueRefIndex(XRc::new(index))
    }
}

impl ValueRef {
    #[inline]
    pub(crate) fn new(lua: &RawLua, ref_id: c_int) -> Self {
        let index_count = ValueRefIndex::from(ref_id);
        ValueRef {
            lua: lua.weak().clone(),
            ref_id,
            index_count: Some(index_count),
        }
    }

    #[inline]
    pub(crate) fn to_pointer(&self) -> *const c_void {
        let lua = self.lua.lock();
        unsafe {
            ffi::lua_getrefpool(lua.state(), self.ref_id);
            let ptr = ffi::lua_topointer(lua.state(), -1);
            ffi::lua_pop(lua.state(), 1);
            ptr
        }
    }
}

impl Drop for ValueRef {
    fn drop(&mut self) {
        if let Some(ValueRefIndex(index)) = self.index_count.take() {
            // It's guaranteed that the inner value returns exactly once.
            // This means in particular that the value is not dropped.
            if XRc::into_inner(index).is_some() {
                if let Some(lua) = self.lua.try_lock() {
                    unsafe { lua.drop_ref(self) };
                }
            }
        }
    }
}

impl fmt::Debug for ValueRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Ref({:p})", self.to_pointer())
    }
}

impl PartialEq for ValueRef {
    fn eq(&self, other: &Self) -> bool {
        assert!(
            self.lua == other.lua,
            "Lua instance passed Value created from a different main Lua state"
        );
        let lua = self.lua.lock();

        unsafe {
            let state = lua.state();
            let _guard = crate::util::StackGuard::new(state);
            ffi::lua_getrefpool(state, self.ref_id);
            ffi::lua_getrefpool(state, other.ref_id);
            ffi::lua_rawequal(state, -1, -2) == 1
        }
    }
}
