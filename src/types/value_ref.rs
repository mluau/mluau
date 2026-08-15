use std::fmt;
use std::os::raw::{c_int, c_void};
use std::cell::Cell;

use super::XRc;
use crate::state::{RawLua, WeakLua};

/// A reference to a Luau (complex) value stored in the Luau refpool.
pub struct ValueRef {
    pub(crate) lua: WeakLua,
    pub(crate) ref_id: c_int,
    count: RefCount,
}

impl ValueRef {
    #[inline]
    pub(crate) fn new(lua: &RawLua, ref_id: c_int) -> Self {
        ValueRef {
            lua: lua.weak().clone(),
            ref_id,
            count: RefCount::unique()
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

impl Clone for ValueRef {
    #[inline]
    fn clone(&self) -> Self {
        ValueRef {
            lua: self.lua.clone(),
            ref_id: self.ref_id,
            count: self.count.clone_shared(),
        }
    }
}

impl Drop for ValueRef {
    fn drop(&mut self) {
        if self.count.drop_is_last() && let Some(lua) = self.lua.try_lock() {
            unsafe { lua.drop_ref(self) }
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

// From mlua-rs/mlua src/types/value_ref.rs -- refcount optimization

// The counter is a pure refcount token.
type Unit = ();
const UNIQUE: *mut Unit = std::ptr::without_provenance_mut(1);

pub(super) struct RefCount(Cell<*mut Unit>);

impl RefCount {
    #[inline]
    pub(super) fn from_raw(ptr: *mut Unit) -> Self {
        RefCount(Cell::new(ptr))
    }

    #[inline]
    pub(super) fn load(&self) -> *mut Unit {
        self.0.get()
    }

    /// Replaces the `UNIQUE` tag with the freshly allocated shared counter `new`.
    ///
    /// Never fails.
    #[inline]
    pub(super) fn promote(&self, new: *mut Unit) {
        debug_assert_eq!(self.0.get(), UNIQUE);
        self.0.set(new);
    }
}

impl RefCount {
    #[inline]
    fn unique() -> Self {
        Self::from_raw(UNIQUE)
    }

    #[inline]
    fn clone_shared(&self) -> RefCount {
        let current = self.load();
        if current != UNIQUE {
            unsafe { XRc::increment_strong_count(current as *const Unit) };
            return RefCount::from_raw(current);
        }

        // Otherwise, lazily allocate the shared counter
        let shared = XRc::into_raw(XRc::new(())) as *mut Unit;
        self.promote(shared);
        unsafe { XRc::increment_strong_count(shared as *const Unit) };
        return RefCount::from_raw(shared);
    }

    /// Drops the reference and returns `true` if it was the last owner of the slot
    /// (so the slot must be freed).
    #[inline]
    fn drop_is_last(&mut self) -> bool {
        let current = self.load();
        if current == UNIQUE {
            true
        } else {
            unsafe { XRc::into_inner(XRc::from_raw(current as *const Unit)).is_some() }
        }
    }
}
