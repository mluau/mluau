#![allow(clippy::await_holding_refcell_ref, clippy::await_holding_lock)]

use std::any::TypeId;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::state::{Lua, LuaGuard};
use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti};
use crate::types::{Callback, MaybeSend};
use crate::userdata::{
    borrow_userdata_scoped, borrow_userdata_scoped_mut, AnyUserData, MetaMethod, TypeIdHints, UserData,
    UserDataFields, UserDataMethods,
};
use crate::util::short_type_name;
use crate::value::Value;

#[cfg(feature = "luau")]
use crate::types::{DynamicCallback, NamecallCallback, XRc};
#[cfg(feature = "luau")]
use std::collections::HashMap;

#[derive(Clone, Copy)]
enum UserDataType {
    Shared(TypeIdHints),
}

/// Handle to registry for userdata methods and metamethods.
pub struct UserDataRegistry<T> {
    lua: LuaGuard,
    raw: RawUserDataRegistry,
    r#type: UserDataType,
    _phantom: PhantomData<T>,
}

pub(crate) struct RawUserDataRegistry {
    // Fields
    pub(crate) fields: Vec<(String, Result<Value>)>,
    pub(crate) field_getters: Vec<(String, Callback)>,
    pub(crate) field_setters: Vec<(String, Callback)>,
    pub(crate) meta_fields: Vec<(String, Result<Value>)>,

    // Functions
    #[cfg(not(feature = "luau"))] // luau has namecalls as a optimization for this
    pub(crate) functions: Vec<(String, Callback)>,
    #[cfg(feature = "luau")]
    pub(crate) functions: Vec<(String, NamecallCallback)>,

    // Methods
    #[cfg(not(feature = "luau"))] // luau has namecalls as a optimization for this
    pub(crate) methods: Vec<(String, Callback)>,
    #[cfg(feature = "luau")]
    pub(crate) methods: Vec<(String, NamecallCallback)>,

    // Metamethods
    pub(crate) meta_methods: Vec<(String, Callback)>,

    pub(crate) destructor: ffi::lua_CFunction,
    pub(crate) type_id: Option<TypeId>,
    pub(crate) type_name: StdString,

    // Namecalls + dynamic methods
    #[cfg(feature = "luau")]
    pub(crate) namecalls: HashMap<String, NamecallCallback>,
    #[cfg(feature = "luau")]
    pub(crate) dynamic_method: Option<DynamicCallback>,
    #[cfg(feature = "luau")]
    pub(crate) disable_namecall_optimization: bool,
}

#[cfg(all(feature = "luau", feature = "send"))]
// SAFETY: The only reason for the non-send is the needed
// clone of the method to both namecalls and methods/functions
//
// This is perfectly safe as we only register within a single
// thread and we do not implement Clone on RawUserDataRegistry
// making it impossible to clone the registry or access its
// methods unsafely
unsafe impl Send for RawUserDataRegistry {}

impl UserDataType {
    #[inline]
    pub(crate) fn type_id(&self) -> Option<TypeId> {
        match self {
            UserDataType::Shared(hints) => Some(hints.type_id()),
        }
    }
}

#[cfg(feature = "send")]
unsafe impl Send for UserDataType {}

impl<T: 'static> UserDataRegistry<T> {
    #[inline(always)]
    pub(crate) fn new(lua: &Lua) -> Self {
        Self::with_type(lua, UserDataType::Shared(TypeIdHints::new::<T>()))
    }
}

impl<T> UserDataRegistry<T> {
    #[inline(always)]
    fn with_type(lua: &Lua, r#type: UserDataType) -> Self {
        let raw = RawUserDataRegistry {
            fields: Vec::new(),
            field_getters: Vec::new(),
            field_setters: Vec::new(),
            meta_fields: Vec::new(),
            functions: Vec::new(),
            methods: Vec::new(),
            meta_methods: Vec::new(),
            destructor: super::util::destroy_userdata_storage::<T>,
            type_id: r#type.type_id(),
            type_name: short_type_name::<T>(),
            #[cfg(feature = "luau")]
            namecalls: HashMap::new(),
            #[cfg(feature = "luau")]
            dynamic_method: None,
            #[cfg(feature = "luau")]
            disable_namecall_optimization: false,
        };

        UserDataRegistry {
            lua: lua.lock_arc(),
            raw,
            r#type,
            _phantom: PhantomData,
        }
    }

    fn box_method<M, A, R>(&self, name: &str, method: M) -> Callback
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let target_type = self.r#type;
        Box::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    #[cfg(feature = "luau")]
    fn box_method_namecall<M, A, R>(&self, name: &str, method: M) -> NamecallCallback
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let target_type = self.r#type;
        XRc::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }

            let state = rawlua.state();

            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    fn box_method_mut<M, A, R>(&self, name: &str, method: M) -> Callback
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let method = RefCell::new(method);
        let target_type = self.r#type;
        Box::new(move |rawlua, nargs| unsafe {
            let mut method = method.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped_mut(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    #[cfg(feature = "luau")]
    fn box_method_mut_namecall<M, A, R>(&self, name: &str, method: M) -> NamecallCallback
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let method = RefCell::new(method);
        let target_type = self.r#type;
        XRc::new(move |rawlua, nargs| unsafe {
            let mut method = method.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped_mut(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    #[cfg(feature = "luau")]
    fn box_dynamic_method<M, A, R>(&self, method: M) -> DynamicCallback
    where
        M: Fn(&Lua, &T, &str, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let target_type = self.r#type;
        XRc::new(move |rawlua, name, nargs| unsafe {
            let name_ref = name;
            macro_rules! try_self_arg {
                ($res:expr) => {
                    $res.map_err(|err| Error::bad_self_argument(&name_ref, err))?
                };
            }

            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }

            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, name, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    #[cfg(feature = "luau")]
    fn box_dynamic_method_mut<M, A, R>(&self, method: M) -> DynamicCallback
    where
        M: FnMut(&Lua, &mut T, &str, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let method = RefCell::new(method);
        let target_type = self.r#type;
        XRc::new(move |rawlua, name, nargs| unsafe {
            let name_ref = name;
            macro_rules! try_self_arg {
                ($res:expr) => {
                    $res.map_err(|err| Error::bad_self_argument(&name_ref, err))?
                };
            }

            let mut method = method.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }

            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_specified_stack_args(nargs - 1, 2, Some(&name), rawlua, state);

            match target_type {
                #[rustfmt::skip]
                UserDataType::Shared(type_hints) => {
                    let type_id = try_self_arg!(rawlua.get_userdata_type_id::<T>(state, self_index));
                    try_self_arg!(borrow_userdata_scoped_mut(state, self_index, type_id, type_hints, |ud| {
                        method(rawlua.lua(), ud, name, args?)?.push_into_specified_stack_multi(rawlua, state)
                    }))
                }
            }
        })
    }

    fn box_function<F, A, R>(&self, name: &str, function: F) -> Callback
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |lua, nargs| unsafe {
            let state = lua.state();
            let args = A::from_specified_stack_args(nargs, 1, Some(&name), lua, state)?;
            function(lua.lua(), args)?.push_into_specified_stack_multi(lua, state)
        })
    }

    #[cfg(feature = "luau")]
    fn box_function_namecall<F, A, R>(&self, name: &str, function: F) -> NamecallCallback
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        XRc::new(move |lua, nargs| unsafe {
            let state = lua.state();
            let args = A::from_specified_stack_args(nargs, 1, Some(&name), lua, state)?;
            function(lua.lua(), args)?.push_into_specified_stack_multi(lua, state)
        })
    }

    fn box_function_mut<F, A, R>(&self, name: &str, function: F) -> Callback
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        let function = RefCell::new(function);
        Box::new(move |lua, nargs| unsafe {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            let state = lua.state();
            let args = A::from_specified_stack_args(nargs, 1, Some(&name), lua, state)?;
            function(lua.lua(), args)?.push_into_specified_stack_multi(lua, state)
        })
    }

    #[cfg(feature = "luau")]
    fn box_function_namecall_mut<F, A, R>(&self, name: &str, function: F) -> NamecallCallback
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        let function = RefCell::new(function);
        XRc::new(move |lua, nargs| unsafe {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            let state = lua.state();
            let args = A::from_specified_stack_args(nargs, 1, Some(&name), lua, state)?;
            function(lua.lua(), args)?.push_into_specified_stack_multi(lua, state)
        })
    }

    pub(crate) fn check_meta_field(lua: &Lua, name: &str, value: impl IntoLua) -> Result<Value> {
        let value = value.into_lua(lua)?;
        if name == MetaMethod::Index || name == MetaMethod::NewIndex {
            match value {
                Value::Nil | Value::Table(_) | Value::Function(_) => {}
                _ => {
                    return Err(Error::MetaMethodTypeError {
                        method: name.to_string(),
                        type_name: value.type_name(),
                        message: Some("expected nil, table or function".to_string()),
                    })
                }
            }
        }
        value.into_lua(lua)
    }

    #[inline(always)]
    pub(crate) fn into_raw(self) -> RawUserDataRegistry {
        self.raw
    }

    /// Sets dynamic method for the userdata type.
    ///
    /// The resulting dynamic method will receive the userdata immutably, along with the method name
    /// and the arguments passed to it.
    ///
    /// This will only override the namecall method for the userdata type, and will
    /// only work with the `:method(...)` syntax, as it uses ``namecall`` under the hood.
    ///
    /// For best user-experience, you should also define a Index metamethod for the userdata type,
    /// which will allow the user to call the method with `data.method(data, ...)` syntax.
    #[cfg(feature = "luau")]
    pub fn set_dynamic_method<F, A, R>(&mut self, method: F)
    where
        F: Fn(&Lua, &T, &str, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let callback = self.box_dynamic_method(method);
        self.raw.dynamic_method = Some(callback);
    }

    /// Sets dynamic mutable method for the userdata type.
    ///
    /// The resulting dynamic method will receive the userdata immutably, along with the method name
    /// and the arguments passed to it.
    ///
    /// This will only override the namecall method for the userdata type, and will
    /// only work with the `:method(...)` syntax, as it uses ``namecall`` under the hood.
    ///
    /// For best user-experience, you should also define a Index metamethod for the userdata type,
    /// which will allow the user to call the method with `data.method(data, ...)` syntax.
    #[cfg(feature = "luau")]
    pub fn set_dynamic_method_mut<F, A, R>(&mut self, method: F)
    where
        F: FnMut(&Lua, &mut T, &str, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let callback = self.box_dynamic_method_mut(method);
        self.raw.dynamic_method = Some(callback);
    }

    /// Disables namecall optimization for the userdata type.
    ///
    /// This will also disable the dynamic method for the userdata type, if it was set (as a side
    /// effect)
    #[cfg(feature = "luau")]
    pub fn disable_namecall_optimization(&mut self) {
        self.raw.disable_namecall_optimization = true;
    }

    /// Returns all fields/methods registered for the userdata type.
    pub fn fields(&self, include_meta: bool) -> Vec<&str> {
        let mut fields = Vec::with_capacity(
            self.raw.fields.len()
                + self.raw.field_getters.len()
                + self.raw.field_setters.len()
                + self.raw.meta_fields.len()
                + self.raw.methods.len()
                + self.raw.meta_methods.len()
                + self.raw.functions.len(),
        );

        for (name, _) in &self.raw.fields {
            fields.push(name.as_str());
        }

        for (name, _) in &self.raw.field_getters {
            fields.push(name.as_str());
        }

        for (name, _) in &self.raw.field_setters {
            fields.push(name.as_str());
        }

        if include_meta {
            for (name, _) in &self.raw.meta_fields {
                fields.push(name.as_str());
            }
        }

        for (name, _) in &self.raw.methods {
            fields.push(name.as_str());
        }

        if include_meta {
            for (name, _) in &self.raw.meta_methods {
                fields.push(name.as_str());
            }
        }

        for (name, _) in &self.raw.functions {
            fields.push(name.as_str());
        }

        fields
    }
}

// Returns function name for the type `T`, without the module path
fn get_function_name<T>(name: &str) -> StdString {
    format!("{}.{name}", short_type_name::<T>())
}

impl<T> UserDataFields<T> for UserDataRegistry<T> {
    fn add_field<V>(&mut self, name: impl Into<StdString>, value: V)
    where
        V: IntoLua + 'static,
    {
        let name = name.into();
        self.raw.fields.push((name, value.into_lua(self.lua.lua())));
    }

    fn add_field_method_get<M, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.into();
        let callback = self.box_method(&name, move |lua, data, ()| method(lua, data));
        self.raw.field_getters.push((name, callback));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.into();
        let callback = self.box_method_mut(&name, method);
        self.raw.field_setters.push((name, callback));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, AnyUserData) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.into();
        let callback = self.box_function(&name, function);
        self.raw.field_getters.push((name, callback));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl Into<StdString>, mut function: F)
    where
        F: FnMut(&Lua, AnyUserData, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.into();
        let callback = self.box_function_mut(&name, move |lua, (data, val)| function(lua, data, val));
        self.raw.field_setters.push((name, callback));
    }

    fn add_meta_field<V>(&mut self, name: impl Into<StdString>, value: V)
    where
        V: IntoLua + 'static,
    {
        let lua = self.lua.lua();
        let name = name.into();
        let field = Self::check_meta_field(lua, &name, value).and_then(|v| v.into_lua(lua));
        self.raw.meta_fields.push((name, field));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl Into<StdString>, f: F)
    where
        F: FnOnce(&Lua) -> Result<R> + 'static,
        R: IntoLua,
    {
        let lua = self.lua.lua();
        let name = name.into();
        let field = f(lua).and_then(|v| Self::check_meta_field(lua, &name, v).and_then(|v| v.into_lua(lua)));
        self.raw.meta_fields.push((name, field));
    }
}

impl<T> UserDataMethods<T> for UserDataRegistry<T> {
    fn add_method<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();

        #[cfg(feature = "luau")]
        {
            let callback = self.box_method_namecall(&name, method);
            self.raw.methods.push((name.clone(), callback.clone()));
            self.raw.namecalls.insert(name, callback);
        }

        #[cfg(not(feature = "luau"))]
        {
            let callback = self.box_method(&name, method);
            self.raw.methods.push((name, callback));
        }
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();

        #[cfg(feature = "luau")]
        {
            let callback = self.box_method_mut_namecall(&name, method);
            self.raw.methods.push((name.clone(), callback.clone()));
            self.raw.namecalls.insert(name, callback);
        }

        #[cfg(not(feature = "luau"))]
        {
            let callback = self.box_method_mut(&name, method);
            self.raw.methods.push((name, callback));
        }
    }

    fn add_function<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        #[cfg(feature = "luau")]
        {
            let name = name.into();
            let callback = self.box_function_namecall(&name, function);
            self.raw.functions.push((name.clone(), callback.clone()));
            self.raw.namecalls.insert(name, callback);
        }
        #[cfg(not(feature = "luau"))]
        {
            let name = name.into();
            let callback = self.box_function(&name, function);
            self.raw.functions.push((name, callback));
        }
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        #[cfg(feature = "luau")]
        {
            let name = name.into();
            let callback = self.box_function_namecall_mut(&name, function);
            self.raw.functions.push((name.clone(), callback.clone()));
            self.raw.namecalls.insert(name, callback);
        }
        #[cfg(not(feature = "luau"))]
        {
            let name = name.into();
            let callback = self.box_function_mut(&name, function);
            self.raw.functions.push((name, callback));
        }
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();
        let callback = self.box_method(&name, method);
        self.raw.meta_methods.push((name, callback));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();
        let callback = self.box_method_mut(&name, method);
        self.raw.meta_methods.push((name, callback));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();
        let callback = self.box_function(&name, function);
        self.raw.meta_methods.push((name, callback));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.into();
        let callback = self.box_function_mut(&name, function);
        self.raw.meta_methods.push((name, callback));
    }
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn register(registry: &mut UserDataRegistry<Self>) {
                let mut orig_registry = UserDataRegistry::new(registry.lua.lua());
                T::register(&mut orig_registry);

                // Copy all fields, methods, etc. from the original registry
                (registry.raw.fields).extend(orig_registry.raw.fields);
                (registry.raw.field_getters).extend(orig_registry.raw.field_getters);
                (registry.raw.field_setters).extend(orig_registry.raw.field_setters);
                (registry.raw.meta_fields).extend(orig_registry.raw.meta_fields);
                (registry.raw.functions).extend(orig_registry.raw.functions);
                (registry.raw.methods).extend(orig_registry.raw.methods);
                (registry.raw.meta_methods).extend(orig_registry.raw.meta_methods);
                #[cfg(feature = "luau")]
                {
                    (registry.raw.namecalls).extend(orig_registry.raw.namecalls);
                    if let Some(dynamic_method) = orig_registry.raw.dynamic_method {
                        registry.raw.dynamic_method = Some(dynamic_method);
                    }
                    registry.raw.disable_namecall_optimization =
                        orig_registry.raw.disable_namecall_optimization;
                }
            }
        }
    };
}

// A special proxy object for UserData
pub(crate) struct UserDataProxy<T>(pub(crate) PhantomData<T>);

lua_userdata_impl!(UserDataProxy<T>);

#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
lua_userdata_impl!(std::rc::Rc<T>);
#[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
lua_userdata_impl!(std::rc::Rc<std::cell::RefCell<T>>);
#[cfg(feature = "userdata-wrappers")]
lua_userdata_impl!(std::sync::Arc<T>);
#[cfg(feature = "userdata-wrappers")]
lua_userdata_impl!(std::sync::Arc<std::sync::Mutex<T>>);
#[cfg(feature = "userdata-wrappers")]
lua_userdata_impl!(std::sync::Arc<std::sync::RwLock<T>>);
#[cfg(feature = "userdata-wrappers")]
lua_userdata_impl!(std::sync::Arc<parking_lot::Mutex<T>>);
#[cfg(feature = "userdata-wrappers")]
lua_userdata_impl!(std::sync::Arc<parking_lot::RwLock<T>>);

#[cfg(test)]
mod assertions {
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(super::RawUserDataRegistry: Send);
}
