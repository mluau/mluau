use std::{cell::RefCell, ffi::CStr};

use crate::{CallbackResult, FromLuaMulti, Function, IntoCallbackResult, Lua};

pub trait FunctionMutExt {
    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`Lua::create_function`] that accepts a `FnMut` argument.
    fn create_function_mut<F, A, R>(&self, func: F) -> crate::Result<Function>
    where
        F: FnMut(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoCallbackResult;

    /// Same as ``create_function_mut`` but with an added ``debugname``
    fn create_function_mut_with_debug<F, A, R>(
        &self,
        func: F,
        debugname: Option<&'static CStr>,
    ) -> crate::Result<Function>
    where
        F: FnMut(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoCallbackResult;
}

impl FunctionMutExt for Lua {
    fn create_function_mut<F, A, R>(&self, func: F) -> crate::Result<Function>
    where
        F: FnMut(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoCallbackResult 
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args: A| {
            let func_ref = func.try_borrow_mut();
            match func_ref {
                Ok(mut r) => (r)(lua, args).into_callback_result(lua),
                Err(_) => CallbackResult::LuaError(crate::Error::RecursiveMutCallback)
            }
        })    
    }

    fn create_function_mut_with_debug<F, A, R>(&self, func: F, debugname: Option<&'static CStr>) -> crate::Result<Function>
    where
        F: FnMut(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoCallbackResult 
    {
        let func = RefCell::new(func);
        self.create_function_with_debug(move |lua, args: A| {
            let func_ref = func.try_borrow_mut();
            match func_ref {
                Ok(mut r) => (r)(lua, args).into_callback_result(lua),
                Err(_) => CallbackResult::LuaError(crate::Error::RecursiveMutCallback)
            }
        }, debugname)    
    }
}
