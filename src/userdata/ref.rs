use std::any::{type_name, TypeId};
use std::ops::{Deref, DerefMut};
use std::os::raw::c_int;
use std::{fmt, mem};

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::traits::FromLua;
use crate::userdata::AnyUserData;
use crate::util::get_userdata;
use crate::value::Value;

use super::cell::UserDataStorage;
use super::lock::{LockGuard, RawLock, UserDataLock};

/// A wrapper type for a userdata value that provides read access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRef<T: 'static> {
    // It's important to drop the guard first, as it refers to the `inner` data.
    _guard: LockGuard<'static, RawLock>,
    inner: UserDataStorage<T>,
}

impl<T> Deref for UserDataRef<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.inner.as_ptr() }
    }
}

impl<T: fmt::Debug> fmt::Debug for UserDataRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for UserDataRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> TryFrom<UserDataStorage<T>> for UserDataRef<T> {
    type Error = Error;

    #[inline]
    fn try_from(variant: UserDataStorage<T>) -> Result<Self> {
        // Shared (read) lock is always correct:
        // - with `send` feature, `T: Sync` is guaranteed by the `MaybeSync` bound on userdata creation
        // - without `send` feature, single-threaded access makes shared lock safe for any `T`
        let guard = variant.raw_lock().try_lock_shared_guarded();
        let guard = guard.map_err(|_| Error::UserDataBorrowError)?;
        let guard = unsafe { mem::transmute::<LockGuard<_>, LockGuard<'static, _>>(guard) };
        Ok(UserDataRef { _guard: guard, inner: variant })
    }
}

impl<T: 'static> FromLua for UserDataRef<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow()
    }

    #[inline]
    unsafe fn from_specified_stack(idx: c_int, lua: &RawLua, state: *mut ffi::lua_State) -> Result<Self> {
        Self::borrow_from_stack(lua, state, idx)
    }
}

impl<T: 'static> UserDataRef<T> {
    // Does not apply to dynamic userdata, as it does not have a type id.
    pub(crate) unsafe fn borrow_from_stack(
        lua: &RawLua,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(state, idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let ud = get_userdata::<UserDataStorage<T>>(state, idx);
                (*ud).try_borrow_owned()
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

/// A wrapper type for a userdata value that provides read and write access.
///
/// It implements [`FromLua`] and can be used to receive a typed userdata from Lua.
pub struct UserDataRefMut<T: 'static> {
    // It's important to drop the guard first, as it refers to the `inner` data.
    _guard: LockGuard<'static, RawLock>,
    inner: UserDataStorage<T>,
}

impl<T> Deref for UserDataRefMut<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.inner.as_ptr() }
    }
}

impl<T> DerefMut for UserDataRefMut<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.as_ptr() }
    }
}

impl<T: fmt::Debug> fmt::Debug for UserDataRefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: fmt::Display> fmt::Display for UserDataRefMut<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T> TryFrom<UserDataStorage<T>> for UserDataRefMut<T> {
    type Error = Error;

    #[inline]
    fn try_from(variant: UserDataStorage<T>) -> Result<Self> {
        let guard = variant.raw_lock().try_lock_exclusive_guarded();
        let guard = guard.map_err(|_| Error::UserDataBorrowMutError)?;
        let guard = unsafe { mem::transmute::<LockGuard<_>, LockGuard<'static, _>>(guard) };
        Ok(UserDataRefMut { _guard: guard, inner: variant })
    }
}

impl<T: 'static> FromLua for UserDataRefMut<T> {
    fn from_lua(value: Value, _: &Lua) -> Result<Self> {
        try_value_to_userdata::<T>(value)?.borrow_mut()
    }

    unsafe fn from_specified_stack(idx: c_int, lua: &RawLua, state: *mut ffi::lua_State) -> Result<Self> {
        Self::borrow_from_stack(lua, state, idx)
    }
}

impl<T: 'static> UserDataRefMut<T> {
    pub(crate) unsafe fn borrow_from_stack(
        lua: &RawLua,
        state: *mut ffi::lua_State,
        idx: c_int,
    ) -> Result<Self> {
        let type_id = lua.get_userdata_type_id::<T>(state, idx)?;
        match type_id {
            Some(type_id) if type_id == TypeId::of::<T>() => {
                let ud = get_userdata::<UserDataStorage<T>>(state, idx);
                (*ud).try_borrow_owned_mut()
            }
            _ => Err(Error::UserDataTypeMismatch),
        }
    }
}

#[inline]
fn try_value_to_userdata<T>(value: Value) -> Result<AnyUserData> {
    match value {
        Value::UserData(ud) => Ok(ud),
        _ => Err(Error::FromLuaConversionError {
            from: value.type_name(),
            to: "userdata".to_string(),
            message: Some(format!("expected userdata of type {}", type_name::<T>())),
        }),
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_not_impl_all!(UserDataRef<std::rc::Rc<()>>: Send, Sync);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(UserDataRefMut<()>: Sync, Send);
    #[cfg(feature = "send")]
    static_assertions::assert_not_impl_all!(UserDataRefMut<std::rc::Rc<()>>: Send, Sync);

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRef<()>: Send, Sync);
    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_all!(UserDataRefMut<()>: Send, Sync);
}
