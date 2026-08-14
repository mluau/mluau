use std::ffi::c_int;
use std::{collections::HashMap, rc::Rc};
use std::marker::PhantomData;
use std::fmt;

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::{AnyUserData, FromLuaMulti, IntoLua, IntoLuaErr, IntoLuaMulti, IntoLuaResult, IntoLuaResultMulti, Lua, MultiValue, Table, TypedUserData, USERDATA2_TAG, Value};

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

impl Into<&'static str> for MetaMethod {
    fn into(self) -> &'static str {
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
pub trait UserDataMethods<T, const TAG: c_int = USERDATA2_TAG> {
    /// Add a regular method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular function which accepts generic arguments.
    ///
    /// The first argument will be a [`AnyUserData`] of type `T` if the method if it is passed in as 
    /// the first argument: `my_userdata.my_method(my_userdata, arg1, arg2)`.
    fn add_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F) where
        F: Fn(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod which accepts a `&T` as the first parameter.
    fn add_meta_method<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod which accepts generic arguments.
    ///
    /// Metamethods for binary operators can be triggered if either the left or right argument to
    /// the binary operator has a metatable, so the first argument here is not necessarily a
    /// userdata of type `T`.
    fn add_meta_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F)
    where
        F: Fn(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;
}

/// Field registry for [`UserData`] implementors.
pub trait UserDataFields<T, const TAG: c_int = USERDATA2_TAG> {
    /// Add a static field to the [`UserData`].
    ///
    /// Static fields are implemented by updating the `__index` metamethod and returning the
    /// accessed field. This allows them to be used with the expected `userdata.field` syntax.
    ///
    /// Static fields are usually shared between all instances of the [`UserData`] of the same type.
    /// 
    /// Note: __index is not an allowed name here for performance purposes, use userdata v2 low-level API instead for that
    fn add_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: IntoLua + 'static;

    /// Add a metatable field.
    ///
    /// This will initialize the metatable field with `value` on [`UserData`] creation.
    /// 
    /// Note: __index is not an allowed name here for performance purposes, use userdata v2 low-level API instead for that
    fn add_meta_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: IntoLua + 'static;

    /// Add a regular field getter as a method which accepts a `&T` as the parameter.
    ///
    /// Regular field getters are implemented by overriding the `__index` metamethod and returning
    /// the accessed field. This allows them to be used with the expected `userdata.field` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular index property is found.
    fn add_field_method_get<M, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>) -> R + 'static,
        R: IntoLuaResult;

    // TODO: Add field method setters
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code.
///
/// Implementation of [`IntoLua`] is automatically provided, Use `TypedUserData<T>` for a [`FromLua`] implementation
/// 
/// Deoptimization notes:
///
/// There are certain cases that may deoptimize userdata accesses into slower paths automatically:
/// - Disabling namecall (forces __index then call as two separate accesses)
/// - Adding a __index metamethod via meta-method (warning: __index as meta function will *not* work right now, this is a current impl limitation, forces all __index to go through func and not table __index)
/// - Field getters (forces func __index)
/// 
/// Mutable Userdata:
/// 
/// - 
pub trait UserData<const TAG: c_int = USERDATA2_TAG>: 'static + Sized {
    /// Whether or not to use __namecall optimization
    /// 
    /// When using namecall optimization:
    /// - Methods must only be added using "add_method". Otherwise method syntax will not work for them
    const USE_NAMECALL: bool = true;

    /// Type name
    fn type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Adds custom fields specific to this userdata.
    #[allow(unused_variables)]
    fn add_fields<F: UserDataFields<Self, TAG>>(fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    #[allow(unused_variables)]
    fn add_methods<M: UserDataMethods<Self, TAG>>(methods: &mut M) {}
}

// Internal impl
pub(super) enum UdError {
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

    fn into_result(self) -> Result<Self::Item, UdError> { self }
}

enum IndexResult {
    Value(Value),
    MultiValue(MultiValue),
}

impl IntoLuaResultMulti for Result<IndexResult, UdError> {
    type Item = MultiValue;
    type Error = UdError;

    fn into_result(self) -> Result<Self::Item, Self::Error> {
        self.map(|x| {
            match x {
                IndexResult::Value(v) => MultiValue::from_iter([v]),
                IndexResult::MultiValue(v) => v
            }
        })
    }
}

type MethodCb<T, const TAG: c_int> = Rc<dyn Fn(&Lua, TypedUserData<T, TAG>, MultiValue) -> Result<MultiValue, UdError> + 'static>;
type FieldGetter<T, const TAG: c_int> = Rc<dyn Fn(&Lua, TypedUserData<T, TAG>) -> Result<Value, UdError> + 'static>;

struct UserDataRegistry<'a, const TAG: c_int, T: UserData<TAG>> {
    lua: &'a Lua,

    // __index props
    index_mt_props: FxHashMap<&'static str, crate::Result<Value>>,
    // main mt props
    mt_props: FxHashMap<&'static str, crate::Result<Value>>,
    
    // special cases
    namecall_methods: FxHashMap<&'static str, MethodCb<T, TAG>>, // Namecall methods (special cases that need namecall optimization)
    namecall_fb: Option<MethodCb<T, TAG>>, // a fallback namecall metamethod
    index_fb: Option<MethodCb<T, TAG>>, // a fallback index metamethod
    field_method_get: FxHashMap<&'static str, FieldGetter<T, TAG>> // fallback index metamethod
}

impl<'a, const TAG: c_int, T: UserData<TAG>> UserDataRegistry<'a, TAG, T> {
    /// Sets up the metatable for a ud
    fn new_metatable(lua: &'a Lua) -> crate::Result<Table> {
        let mut reg = Self {
            lua,
            index_mt_props: FxHashMap::default(),
            mt_props: FxHashMap::default(),
            namecall_methods: FxHashMap::default(),
            namecall_fb: None,
            index_fb: None,
            field_method_get: FxHashMap::default(),
        };

        // Collect into reg
        T::add_fields(&mut reg);
        T::add_methods(&mut reg);

        reg.finalize_metatable()
    }

    fn finalize_metatable(self) -> crate::Result<Table> {
        // Metatable vecs, we can then build the 2 tables for __index and main mt in one shot later
        let mut mt = Vec::with_capacity(self.mt_props.len());

        // Setup __index as either table or fn
        let index_as_fn = self.index_fb.is_some() || !self.field_method_get.is_empty();
        let index = if index_as_fn {
            // Build up the index_mt_props hashmap with values resolved
            let index_mt_props = Rc::new({
                let mut new_index_mt_props = HashMap::with_capacity_and_hasher(self.index_mt_props.len(), FxBuildHasher);
                for (k, v) in self.index_mt_props {
                    new_index_mt_props.insert(k, v?);
                }
                new_index_mt_props
            });
            let index_fb = self.index_fb;
            let field_getters = self.field_method_get;
            let indexfn = self.lua.create_function_with_debug(move |lua, (ud, key): (TypedUserData<T, TAG>, crate::String)| {
                let key_str = key.to_str().map_err(UdError::Error)?;
                // Case 1: prop is in index_mt_props directly
                if let Some(prop) = index_mt_props.get(key_str.as_ref()) {
                    return Ok(IndexResult::Value(prop.clone()))
                }
                // Case 2: field getter
                if let Some(ref getter) = field_getters.get(key_str.as_ref()) {
                    return getter(lua, ud).map(IndexResult::Value)
                }
                // Lastly, check for custom index
                if let Some(ref ifb) = index_fb {
                    return ifb(lua, ud, MultiValue::from_iter([Value::String(key)])).map(IndexResult::MultiValue)
                }
                Ok(IndexResult::Value(Value::Nil))
            }, Some(c"__index"))?;

            Value::Function(indexfn)
        } else {
            let mut indexmt = Vec::with_capacity(self.index_mt_props.len());
            for (k, v) in self.index_mt_props {
                indexmt.push((k, v?));
            }
            indexmt.push(("__metatable", Value::Boolean(false)));
            let indextab = self.lua.create_table_from(indexmt)?;
            indextab.set_readonly(true);
            Value::Table(indextab)
        };

        // Setup main __mt
        for (k, v) in self.mt_props {
            mt.push((k, v?));
        }
        let mt = self.lua.create_table_from(mt)?;
        
        mt.set("__index", index)?;
        
        // Inject __type if not explicitly set
        if !mt.raw_get("__type")? {
            mt.set("__type", T::type_name())?;
        }

        if T::USE_NAMECALL {
            let namecall_cbs = Rc::new(self.namecall_methods);
            mt.set("__namecall", Self::namecall_cb(self.lua, namecall_cbs, self.namecall_fb)?)?;
        }

        mt.set("__metatable", false)?;
        mt.set_readonly(true);

        Ok(mt)
    }

    fn namecall_cb(lua: &Lua, namecall_methods: Rc<FxHashMap<&'static str, MethodCb<T, TAG>>>, namecall_fb: Option<MethodCb<T, TAG>>) -> crate::Result<crate::Function> {
        lua.create_function(move |lua, (ud, args): (TypedUserData<T, TAG>, MultiValue)| {
            let func = lua.with_namecall(|method| -> Result<_, crate::Error> {
                let s = method.ok_or_else(|| crate::Error::external("internal error: no method set for namecall"))?.to_str().map_err(crate::Error::external)?;
                match namecall_methods.get(s) {
                    Some(s) => Ok(s.clone()),
                    None => {
                        if let Some(fb) = &namecall_fb {
                            Ok(fb.clone())   
                        } else {
                            Err(crate::Error::runtime(format!("{}: cannot find method `{s}`", T::type_name())))
                        }
                    }
                }
            }).map_err(UdError::Error)?;
            func(lua, ud, args)
        })
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
            return lua.create_any_userdata_with_tag::<_, TAG>(data, Some(&tab.tab));
        }

        let mt = Self::new_metatable(lua)?;
        lua.set_app_data(Ud {
            tab: mt.clone(),
            _phantom: PhantomData::<T>
        });
        lua.create_any_userdata_with_tag::<_, TAG>(data, Some(&mt))
    }
}

impl<const TAG: c_int, T: UserData<TAG>> UserDataFields<T, TAG> for UserDataRegistry<'_, TAG, T> {
    fn add_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where V: IntoLua + 'static 
    {
        self.index_mt_props.insert(name.into(), value.into_lua(self.lua));
    }

    fn add_meta_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
    V: IntoLua + 'static 
    {
        self.mt_props.insert(name.into(), value.into_lua(self.lua));
    }

    fn add_field_method_get<M, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>) -> R + 'static,
        R: IntoLuaResult
    {
        self.field_method_get.insert(name.into(), wrap_field_getter::<TAG, T, M, R>(method));
    }
}

impl<const TAG: c_int, T: UserData<TAG>> UserDataMethods<T, TAG> for UserDataRegistry<'_, TAG, T> {
    fn add_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F) where
    F: Fn(&Lua, A) -> R + 'static,
    A: FromLuaMulti,
    R: IntoLuaResultMulti 
    {
        self.index_mt_props.insert(name.into(), self.lua.create_function(function).map(Value::Function));
    }

    fn add_method<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        // Methods are a bit special due to __namecall optimization which normal functions don't get
        //
        // If namecall optimization is enabled, we need to wrap methods to get a namecall repr for __namecall optimization
        let wrapped = wrap_method(method);
        let name = name.into();
        self.namecall_methods.insert(name, wrapped.clone());
        self.index_mt_props.insert(name, self.lua.create_function(move |lua: &Lua, (ud, args): (TypedUserData<T, TAG>, MultiValue)| {
            wrapped(lua, ud, args)
        }).map(Value::Function));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F) where
    F: Fn(&Lua, A) -> R + 'static,
    A: FromLuaMulti,
    R: IntoLuaResultMulti 
    {
        self.mt_props.insert(name.into(), self.lua.create_function(function).map(Value::Function));
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, TypedUserData<T, TAG>, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        let name = name.into();
        match name {
            "__index" => { self.index_fb = Some(wrap_method(method)); },
            "__namecall" => { self.namecall_fb = Some(wrap_method(method)); },
            _ => { self.mt_props.insert(name, self.lua.create_function(move |lua, (ud, args)| method(lua, ud, args)).map(Value::Function)); }
        };
    }
}

impl<T: UserData> IntoLua for T {
    #[inline]
    fn into_lua(self, lua: &Lua) -> crate::Result<Value> {
        let ud = UserDataRegistry::create_userdata(lua, self)?;
        Ok(Value::UserData(ud))
    }
}

pub trait LuaUserDataExt {
    fn create_userdata<const TAG: c_int, T: UserData<TAG>>(&self, data: T) -> crate::Result<AnyUserData>;
    /// Enriches a AnyUserData of mutable userdata of base type `T` into the underlying `T`
    /// 
    /// Will return `None` if `T` if not the type of the data within the AnyUserData
    fn into<const TAG: c_int, T: UserData<TAG>>(self, ud: AnyUserData) -> Option<TypedUserData<T, TAG>>;
}

impl LuaUserDataExt for Lua {
    fn create_userdata<const TAG: c_int, T: UserData<TAG>>(&self, data: T) -> crate::Result<AnyUserData> {
        let ud = UserDataRegistry::create_userdata(self, data)?;
        Ok(ud)
    }
    fn into<const TAG: c_int, T: UserData<TAG>>(self, ud: AnyUserData) -> Option<TypedUserData<T, TAG>> {
        ud.into_with_tag::<T, TAG>()
    }
}

// Helpers for wrapping fn's/cb's

#[inline]
/// Wraps a method into a type-erased MethodCb<T>
pub(super) fn wrap_method<const TAG: c_int, T, M, A, R>(method: M) -> MethodCb<T, TAG>
where
    T: UserData<TAG>,
    M: Fn(&Lua, TypedUserData<T, TAG>, A) -> R + 'static,
    A: FromLuaMulti,
    R: IntoLuaResultMulti,
{
    Rc::new(move |lua, this, args| {
        let args = match A::from_lua_multi(args, lua) {
            Ok(a) => a,
            Err(e) => return Err(UdError::Error(e)),
        };
        
        // Call method and normalize
        match method(lua, this, args).into_result() {
            Ok(item) => item.into_lua_multi(lua).map_err(UdError::Error),
            Err(err) => match err.into_lua_err(lua) {
                Ok(v) => Err(UdError::Value(v)),
                Err(e) => Err(UdError::Error(e)),
            }
        }
    })
}

#[inline]
/// Wraps a method into a type-erased MethodCb<T>
pub(super) fn wrap_field_getter<const TAG: c_int, T, M, R>(method: M) -> FieldGetter<T, TAG>
where
    T: UserData<TAG>,
    M: Fn(&Lua, TypedUserData<T, TAG>) -> R + 'static,
    R: IntoLuaResult,
{
    Rc::new(move |lua, this| {
        // Call method and normalize
        match method(lua, this).into_result() {
            Ok(item) => item.into_lua(lua).map_err(UdError::Error),
            Err(err) => match err.into_lua_err(lua) {
                Ok(v) => Err(UdError::Value(v)),
                Err(e) => Err(UdError::Error(e)),
            }
        }
    })
}
