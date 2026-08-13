use std::marker::PhantomData;
use std::fmt;

use crate::types::MaybeSync;
use crate::{AnyUserData, FromLuaMulti, IntoLua, IntoLuaErr, IntoLuaMulti, IntoLuaResultMulti, Lua, MaybeSend, MultiValue, Table, Value, XRc};

/// Kinds of metamethods that can be overridden.
///
/// Currently, this mechanism does not allow overriding the `__gc` metamethod, since there is
/// generally no need to do so: [`UserData`] implementors can instead just implement `Drop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MetaMethod {
    /// The `+` operator.
    Add,
    /// The `-` operator.
    Sub,
    /// The `*` operator.
    Mul,
    /// The `/` operator.
    Div,
    /// The `%` operator.
    Mod,
    /// The `^` operator.
    Pow,
    /// The unary minus (`-`) operator.
    Unm,
    /// The floor division (//) operator.
    IDiv,
    /// The string concatenation operator `..`.
    Concat,
    /// The length operator `#`.
    Len,
    /// The `==` operator.
    Eq,
    /// The `<` operator.
    Lt,
    /// The `<=` operator.
    Le,
    /// Index access `obj[key]`.
    Index,
    /// Index write access `obj[key] = value`.
    NewIndex,
    /// The call "operator" `obj(arg1, args2, ...)`.
    Call,
    /// The `__tostring` metamethod.
    ///
    /// This is not an operator, but will be called by methods such as `tostring` and `print`.
    ToString,
    /// The `__iter` metamethod.
    ///
    /// Executed before the iteration begins, and should return an iterator function like `next`
    /// (or a custom one).
    Iter,
    /// The `__type` metafield.
    ///
    /// This is not a function, but it's value can be used by `tostring` and `typeof` built-in
    /// functions.
    #[doc(hidden)]
    Type,
}

impl PartialEq<MetaMethod> for &str {
    fn eq(&self, other: &MetaMethod) -> bool {
        *self == other.name()
    }
}

impl PartialEq<MetaMethod> for String {
    fn eq(&self, other: &MetaMethod) -> bool {
        self == other.name()
    }
}

impl fmt::Display for MetaMethod {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "{}", self.name())
    }
}

impl MetaMethod {
    /// Returns Lua metamethod name, usually prefixed by two underscores.
    pub const fn name(self) -> &'static str {
        match self {
            MetaMethod::Add => "__add",
            MetaMethod::Sub => "__sub",
            MetaMethod::Mul => "__mul",
            MetaMethod::Div => "__div",
            MetaMethod::Mod => "__mod",
            MetaMethod::Pow => "__pow",
            MetaMethod::Unm => "__unm",
            MetaMethod::IDiv => "__idiv",
            MetaMethod::Concat => "__concat",
            MetaMethod::Len => "__len",
            MetaMethod::Eq => "__eq",
            MetaMethod::Lt => "__lt",
            MetaMethod::Le => "__le",
            MetaMethod::Index => "__index",
            MetaMethod::NewIndex => "__newindex",
            MetaMethod::Call => "__call",
            MetaMethod::ToString => "__tostring",

            MetaMethod::Iter => "__iter",

            #[rustfmt::skip]
            MetaMethod::Type => "__type",
        }
    }
}

impl AsRef<str> for MetaMethod {
    fn as_ref(&self) -> &str {
        self.name()
    }
}

impl From<MetaMethod> for String {
    #[inline]
    fn from(method: MetaMethod) -> Self {
        method.name().to_owned()
    }
}

/// Method registry for [`UserData`] implementors.
pub trait UserDataMethods<T> {
    /// Add a regular method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<M, A, R>(&mut self, name: &'static str, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular function which accepts generic arguments.
    ///
    /// The first argument will be a [`AnyUserData`] of type `T` if the method if it is passed in as 
    /// the first argument: `my_userdata.my_method(my_userdata, arg1, arg2)`.
    fn add_function<F, A, R>(&mut self, name: &'static str, function: F) where
        F: Fn(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod which accepts a `&T` as the first parameter.
    /// 
    /// Note: __index is not an allowed name here for performance purposes, use userdata v2 low-level API instead for that
    fn add_meta_method<M, A, R>(&mut self, name: &'static str, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;
}

/// Field registry for [`UserData`] implementors.
pub trait UserDataFields<T> {
    /// Add a static field to the [`UserData`].
    ///
    /// Static fields are implemented by updating the `__index` metamethod and returning the
    /// accessed field. This allows them to be used with the expected `userdata.field` syntax.
    ///
    /// Static fields are usually shared between all instances of the [`UserData`] of the same type.
    /// 
    /// Note: __index is not an allowed name here for performance purposes, use userdata v2 low-level API instead for that
    fn add_field<V>(&mut self, name: &'static str, value: V)
    where
        V: IntoLua + 'static;

    /// Add a metatable field.
    ///
    /// This will initialize the metatable field with `value` on [`UserData`] creation.
    /// 
    /// Note: __index is not an allowed name here for performance purposes, use userdata v2 low-level API instead for that
    fn add_meta_field<V>(&mut self, name: &'static str, value: V)
    where
        V: IntoLua + 'static;
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code.
///
/// Implementation of [`IntoLua`] is automatically provided, [`FromLua`] needs to be implemented
/// manually.
pub trait UserData: 'static + Sized + MaybeSend + MaybeSync {
    /// Type name
    fn type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Adds custom fields specific to this userdata.
    #[allow(unused_variables)]
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    #[allow(unused_variables)]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {}
}

// Internal impl

enum UdError {
    Value(Value),
    Error(crate::Error)
}

impl IntoLuaErr for UdError {
    fn into_lua_err(self, lua: &Lua) -> crate::Result<Value> {
        match self {
            UdError::Value(v) => Ok(v),
            UdError::Error(e) => e.into_lua_err(lua),
        }
    }
}

impl IntoLuaResultMulti for Result<MultiValue, UdError> {
    type Item = MultiValue;
    type Error = UdError;

    fn into_result(self) -> std::result::Result<Self::Item, UdError> { self }
}

type FnCb = Box<dyn Fn(&Lua, MultiValue) -> Result<MultiValue, UdError> + 'static>;
type MethodCb<T> = XRc<dyn Fn(&Lua, crate::LuaRef<'_, T>, MultiValue) -> Result<MultiValue, UdError> + 'static>;

struct UserDataRegistry<'a, T: UserData> {
    lua: &'a Lua,

    // Fields
    fields: Vec<(&'static str, crate::Result<Value>)>,
    meta_fields: Vec<(&'static str, crate::Result<Value>)>,
    
    // Methods
    functions: Vec<(&'static str, FnCb)>,
    methods: Vec<(&'static str, MethodCb<T>)>,
    meta_methods: Vec<(&'static str, MethodCb<T>)>,
}

impl<'a, T: UserData> UserDataRegistry<'a, T> {
    fn new_metatable(lua: &'a Lua) -> crate::Result<Table> {
        let mut reg = Self {
            lua,
            fields: Vec::new(),
            meta_fields: Vec::new(),
            functions: Vec::new(),
            methods: Vec::new(),
            meta_methods: Vec::new(),
        };

        // Collect into reg
        T::add_fields(&mut reg);
        T::add_methods(&mut reg);

        reg.finalize_metatable(lua)
    }

    fn len(&self) -> usize {
        self.fields.len() + 
        self.meta_fields.len() + 
        self.functions.len() +
        self.methods.len() +
        self.meta_methods.len()
    }

    #[inline]
    fn wrap_method<M, A, R>(method: M) -> MethodCb<T>
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        XRc::new(move |lua, this, args| {
            let args = match A::from_lua_multi(args, lua) {
                Ok(a) => a,
                Err(e) => return Err(UdError::Error(e)),
            };
            
            // Call method and normalize
            match method(lua, &*this, args).into_result() {
                Ok(item) => item.into_lua_multi(lua).map_err(UdError::Error),
                Err(err) => match err.into_lua_err(lua) {
                    Ok(v) => Err(UdError::Value(v)),
                    Err(e) => Err(UdError::Error(e)),
                }
            }
        })
    }
    
    #[inline]
    fn wrap_function<F, A, R>(function: F) -> FnCb
    where
        F: Fn(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        Box::new(move |lua, args| {
            let args = match A::from_lua_multi(args, lua) {
                Ok(a) => a,
                Err(e) => return Err(UdError::Error(e)),
            };
    
            // Call function and normalize
            match function(lua, args).into_result() {
                Ok(item) => item.into_lua_multi(lua).map_err(UdError::Error),
                Err(err) => match err.into_lua_err(lua) {
                    Ok(v) => Err(UdError::Value(v)),
                    Err(e) => Err(UdError::Error(e)),
                }
            }
        })
    }

    fn finalize_metatable(self, lua: &Lua) -> crate::Result<Table> {
        // __index mt
        let mut indexmt = Vec::with_capacity(self.len());
        let mut mt = Vec::with_capacity(self.len());

        // Fields, functions go into __index
        for (k, v) in self.fields {
            indexmt.push((k, v?));
        }
        for (k, v) in self.functions {
            let func = lua.create_function(v)?;
            indexmt.push((k, Value::Function(func)));
        }
        for (k, v) in self.methods {
            let func = lua.create_userdata_method(move |lua: &Lua, this: crate::LuaRef<'_, T>, args: MultiValue| {
                v(lua, this, args)
            }, None, T::type_name(), k)?;
            indexmt.push((k, Value::Function(func)));
        }

        // Metafields, metamethods into metatable directly
        for (k, v) in self.meta_fields {
            if k == "__index" {
                return Err(crate::Error::external("__index metamethod cannot be set with userdata aux api"));
            }
            mt.push((k, v?));
        }
        for (k, v) in self.meta_methods {
            if k == "__index" {
                return Err(crate::Error::external("__index metamethod cannot be set with userdata aux api"));
            }
            let func = lua.create_userdata_method(move |lua: &Lua, this: crate::LuaRef<'_, T>, args: MultiValue| {
                v(lua, this, args)
            }, None, T::type_name(), k)?;
            mt.push((k, Value::Function(func)));
        }


        // Finalize
        let indextab = lua.create_table_from(indexmt)?;
        indextab.set_readonly(true);

        let mt = lua.create_table_from(mt)?;
        
        mt.set("__index", indextab)?;
        mt.set_readonly(true);

        Ok(mt)
    }

    // TODO: Make this re-entrant/multiple threads safe

    /// Creates the actual userdata w/ shared metatable for all `T` of same type
    fn create_userdata(lua: &'a Lua, data: T) -> crate::Result<AnyUserData> {
        // Check if `ud` already exists
        struct Ud<T> {
            tab: Table,
            _phantom: PhantomData<T>
        }
        if let Some(tab) = lua.try_app_data_ref::<Ud<T>>().map_err(crate::Error::external)? {
            return lua.create_any_userdata(data, Some(&tab.tab));
        }

        let mt = Self::new_metatable(lua)?;
        lua.set_app_data(Ud {
            tab: mt.clone(),
            _phantom: PhantomData::<T>
        });
        lua.create_any_userdata(data, Some(&mt))
    }
}

impl<T: UserData> UserDataFields<T> for UserDataRegistry<'_, T> {
    fn add_field<V>(&mut self, name: &'static str, value: V)
    where V: IntoLua + 'static 
    {
        self.fields.push((name, value.into_lua(self.lua)))
    }

    fn add_meta_field<V>(&mut self, name: &'static str, value: V)
    where
    V: IntoLua + 'static 
    {
        self.meta_fields.push((name, value.into_lua(self.lua)))
    }
}

impl<T: UserData> UserDataMethods<T> for UserDataRegistry<'_, T> {
    fn add_function<F, A, R>(&mut self, name: &'static str, function: F) where
    F: Fn(&Lua, A) -> R + MaybeSend + 'static,
    A: FromLuaMulti,
    R: IntoLuaResultMulti 
    {
        let wrapped = Self::wrap_function(function);
        self.functions.push((name, wrapped))
    }

    fn add_method<M, A, R>(&mut self, name: &'static str, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        let wrapped = Self::wrap_method(method);
        self.methods.push((name, wrapped)) 
    }

    fn add_meta_method<M, A, R>(&mut self, name: &'static str, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        let wrapped = Self::wrap_method(method);
        self.meta_methods.push((name, wrapped)) 
    }
}

// Conversion impls
impl<T: UserData> IntoLua for T {
    #[inline]
    fn into_lua(self, lua: &Lua) -> crate::Result<Value> {
        let ud = UserDataRegistry::create_userdata(lua, self)?;
        Ok(Value::UserData(ud))
    }
}