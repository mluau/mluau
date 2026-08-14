use std::os::raw::c_int;
use std::string::String as StdString;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::multi::MultiValue;
use crate::state::{Lua, RawLua};
use crate::util::{check_stack, short_type_name};
use crate::value::Value;

/// Trait for types convertible to a Lua error value.
pub trait IntoLuaErr: Sized {
    /// Performs the conversion.
    fn into_lua_err(self, lua: &Lua) -> Result<Value>;
}

pub trait IntoLuaResultMulti {
    type Item: IntoLuaMulti;
    type Error: crate::traits::IntoLuaErr;
    fn into_result(self) -> std::result::Result<Self::Item, Self::Error>;
}

// For backwards compat, we only impl IntoLuaResult for Result<T, mluau::Error>
impl<T: IntoLuaMulti> IntoLuaResultMulti for std::result::Result<T, crate::Error> {
    type Item = T;
    type Error = crate::Error;
    fn into_result(self) -> std::result::Result<Self::Item, Self::Error> { self }
}

pub trait IntoLuaResult {
    type Item: IntoLua;
    type Error: crate::traits::IntoLuaErr;
    fn into_result(self) -> std::result::Result<Self::Item, Self::Error>;
}

impl<T: IntoLua> IntoLuaResult for std::result::Result<T, crate::Error> {
    type Item = T;
    type Error = crate::Error;
    fn into_result(self) -> std::result::Result<Self::Item, Self::Error> { self }
}

/// Trait for types convertible to [`Value`].
pub trait IntoLua: Sized {
    /// Performs the conversion.
    fn into_lua(self, lua: &Lua) -> Result<Value>;

    /// Pushes the value directly into a Lua stack
    ///
    /// # Safety
    /// This method does not check Lua stack space.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_specified_stack(self, lua: &RawLua, state: *mut ffi::lua_State) -> Result<()> {
        lua.push_value_at(&self.into_lua(lua.lua())?, state);
        Ok(())
    }
}

/// Trait for types convertible from [`Value`].
pub trait FromLua: Sized {
    /// Performs the conversion.
    fn from_lua(value: Value, lua: &Lua) -> Result<Self>;

    /// Performs the conversion for an argument (eg. function argument).
    ///
    /// `i` is the argument index (position),
    /// `to` is a function name that received the argument.
    #[doc(hidden)]
    #[inline]
    fn from_lua_arg(arg: Value, i: usize, to: Option<&str>, lua: &Lua) -> Result<Self> {
        Self::from_lua(arg, lua).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }

    /// Performs the conversion for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_specified_stack(idx: c_int, lua: &RawLua, state: *mut ffi::lua_State) -> Result<Self> {
        Self::from_lua(lua.stack_value_at(idx, None, state)?, lua.lua())
    }

    /// Same as `from_lua_arg` but for a value in the Lua stack at index `idx`.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_specified_stack_arg(
        idx: c_int,
        i: usize,
        to: Option<&str>,
        lua: &RawLua,
        state: *mut ffi::lua_State,
    ) -> Result<Self> {
        Self::from_specified_stack(idx, lua, state).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }
}

/// Trait for types convertible to any number of Lua values.
///
/// This is a generalization of [`IntoLua`], allowing any number of resulting Lua values instead of
/// just one. Any type that implements [`IntoLua`] will automatically implement this trait.
pub trait IntoLuaMulti: Sized {
    /// Performs the conversion.
    fn into_lua_multi(self, lua: &Lua) -> Result<MultiValue>;

    /// Pushes the values directly into a Lua stack
    ///
    /// Returns number of pushed values.
    #[doc(hidden)]
    #[inline]
    unsafe fn push_into_specified_stack_multi(
        self,
        lua: &RawLua,
        state: *mut ffi::lua_State,
    ) -> Result<c_int> {
        let values = self.into_lua_multi(lua.lua())?;
        let len: c_int = values.len().try_into().unwrap();
        unsafe {
            check_stack(state, len + 1)?;
            for val in &values {
                lua.push_value_at(val, state);
            }
        }
        Ok(len)
    }
}

/// Trait for types that can be created from an arbitrary number of Lua values.
///
/// This is a generalization of [`FromLua`], allowing an arbitrary number of Lua values to
/// participate in the conversion. Any type that implements [`FromLua`] will automatically
/// implement this trait.
pub trait FromLuaMulti: Sized {
    /// Performs the conversion.
    ///
    /// In case `values` contains more values than needed to perform the conversion, the excess
    /// values should be ignored. This reflects the semantics of Lua when calling a function or
    /// assigning values. Similarly, if not enough values are given, conversions should assume that
    /// any missing values are nil.
    fn from_lua_multi(values: MultiValue, lua: &Lua) -> Result<Self>;

    /// Performs the conversion for a list of arguments.
    ///
    /// `i` is an index (position) of the first argument,
    /// `to` is a function name that received the arguments.
    #[doc(hidden)]
    #[inline]
    fn from_lua_args(args: MultiValue, i: usize, to: Option<&str>, lua: &Lua) -> Result<Self> {
        let _ = (i, to);
        Self::from_lua_multi(args, lua)
    }

    /// Performs the conversion for a number of values in the specified Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_specified_stack_multi(
        nvals: c_int,
        lua: &RawLua,
        state: *mut ffi::lua_State,
    ) -> Result<Self> {
        let mut values = MultiValue::with_capacity(nvals as usize);
        for idx in 0..nvals {
            values.push_back(lua.stack_value_at(-nvals + idx, None, state)?);
        }
        Self::from_lua_multi(values, lua.lua())
    }

    /// Same as `from_lua_args` but for a number of values in the specified Lua stack.
    #[doc(hidden)]
    #[inline]
    unsafe fn from_specified_stack_args(
        nvals: c_int,
        i: usize,
        to: Option<&str>,
        lua: &RawLua,
        state: *mut ffi::lua_State,
    ) -> Result<Self> {
        let _ = (i, to);
        Self::from_specified_stack_multi(nvals, lua, state).map_err(|err| Error::BadArgument {
            to: to.map(|s| s.to_string()),
            pos: i,
            name: None,
            cause: Arc::new(err),
        })
    }
}

/// A trait for types that can be used as Lua functions.
pub trait LuaNativeFn<A: FromLuaMulti> {
    type Output;

    fn call(&self, args: A) -> Self::Output;
}

/// A trait for types with mutable state that can be used as Lua functions.
pub trait LuaNativeFnMut<A: FromLuaMulti> {
    type Output;

    fn call(&mut self, args: A) -> Self::Output;
}

macro_rules! impl_lua_native_fn {
    ($($A:ident),*) => {
        impl<FN, $($A,)* R> LuaNativeFn<($($A,)*)> for FN
        where
            FN: Fn($($A,)*) -> R + 'static,
            ($($A,)*): FromLuaMulti,
        {
            type Output = R;

            #[allow(non_snake_case)]
            fn call(&self, args: ($($A,)*)) -> Self::Output {
                let ($($A,)*) = args;
                self($($A,)*)
            }
        }

        impl<FN, $($A,)* R> LuaNativeFnMut<($($A,)*)> for FN
        where
            FN: FnMut($($A,)*) -> R + 'static,
            ($($A,)*): FromLuaMulti,
        {
            type Output = R;

            #[allow(non_snake_case)]
            fn call(&mut self, args: ($($A,)*)) -> Self::Output {
                let ($($A,)*) = args;
                self($($A,)*)
            }
        }
    };
}

impl_lua_native_fn!();
impl_lua_native_fn!(A);
impl_lua_native_fn!(A, B);
impl_lua_native_fn!(A, B, C);
impl_lua_native_fn!(A, B, C, D);
impl_lua_native_fn!(A, B, C, D, E);
impl_lua_native_fn!(A, B, C, D, E, F);
impl_lua_native_fn!(A, B, C, D, E, F, G);
impl_lua_native_fn!(A, B, C, D, E, F, G, H);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_lua_native_fn!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

pub(crate) trait ShortTypeName {
    #[inline(always)]
    fn type_name() -> StdString {
        short_type_name::<Self>()
    }
}

impl<T> ShortTypeName for T {}
