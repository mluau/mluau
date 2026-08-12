use std::any::TypeId;
use std::ffi::CStr;
use std::fmt;
use std::hash::Hash;
use std::os::raw::c_void;
use std::string::String as StdString;

use crate::{IntoLuaMulti, WeakLua};
use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::Lua;
use crate::string::String;
use crate::table::{Table, TablePairs};
use crate::traits::{FromLua, FromLuaMulti, IntoLua, IntoLuaResult, IntoLuaResultMulti};
use crate::types::{MaybeSend, MaybeSync, ValueRef};
use crate::util::{check_stack, get_userdata, take_userdata, StackGuard};
use crate::value::Value;



// Re-export for convenience
pub(crate) use cell::UserDataStorage;
pub use r#ref::{UserDataRef, UserDataRefMut};
#[cfg(feature = "dynamic-userdata")]
pub(crate) use registry::DynamicUserDataPtr;
pub use registry::UserDataRegistry;
pub(crate) use registry::RawUserDataRegistry;
#[cfg(feature = "dynamic-userdata")]
pub(crate) use util::collect_userdata_dyn;
pub(crate) use util::{
    borrow_userdata_scoped, borrow_userdata_scoped_mut, collect_userdata, init_userdata_metatable,
    TypeIdHints,
};

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

    /// The `__name`/`__type` metafield.
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

impl PartialEq<MetaMethod> for StdString {
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

            MetaMethod::Type => "__type",
        }
    }

    pub(crate) const fn as_cstr(self) -> &'static CStr {
        match self {
            #[rustfmt::skip]
            MetaMethod::Type => if cfg!(feature = "luau") { c"__type" } else { c"__name" },
            _ => unreachable!(),
        }
    }

    pub(crate) fn validate(name: &str) -> Result<&str> {
        match name {
            // __gc is safe on Luau as it doesnt actually exist
            "__metatable" => Err(Error::MetaMethodRestricted(name.to_string())),
            _ if name.starts_with("__mlua") => Err(Error::MetaMethodRestricted(name.to_string())),
            name => Ok(name),
        }
    }
}

impl AsRef<str> for MetaMethod {
    fn as_ref(&self) -> &str {
        self.name()
    }
}

impl From<MetaMethod> for StdString {
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
    fn add_method<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Same as add_method but with added support for a static debugname for the method.
    ///
    /// Refer to [`add_method`] for more information about the implementation.
    ///
    /// Will disable namecall optimization if enabled
    ///
    /// [`add_method`]: UserDataMethods::add_method

    fn add_method_with_debug<M, A, R>(
        &mut self,
        name: impl Into<StdString>,
        debugname: &'static CStr,
        method: M,
    ) where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular method which accepts a `&mut T` as the first parameter.
    ///
    /// Refer to [`add_method`] for more information about the implementation.
    ///
    /// [`add_method`]: UserDataMethods::add_method
    fn add_method_mut<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Same as add_method_mut but with added support for a static debugname for the method.
    ///
    /// Refer to [`add_method_mut`] for more information about the implementation.
    ///
    /// Will disable namecall optimization if enabled
    ///
    /// [`add_method_mut`]: UserDataMethods::add_method_mut

    fn add_method_mut_with_debug<M, A, R>(
        &mut self,
        name: impl Into<StdString>,
        debugname: &'static CStr,
        method: M,
    ) where
        M: FnMut(&Lua, &mut T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular method as a function which accepts generic arguments.
    ///
    /// The first argument will be a [`AnyUserData`] of type `T` if the method is called with Lua
    /// method syntax: `my_userdata:my_method(arg1, arg2)`, or it is passed in as the first
    /// argument: `my_userdata.my_method(my_userdata, arg1, arg2)`.
    fn add_function<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular method as a function with debugname which accepts generic arguments
    ///
    /// This is a version of [`add_function`] that accepts a debug name
    ///
    /// [`add_function`]: UserDataMethods::add_function

    fn add_function_with_debug<F, A, R>(
        &mut self,
        name: impl Into<StdString>,
        debugname: &'static CStr,
        function: F,
    ) where
        F: Fn(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular method as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_function`] that accepts a `FnMut` argument.
    ///
    /// [`add_function`]: UserDataMethods::add_function
    fn add_function_mut<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: FnMut(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a regular method as a mutable function with debugname which accepts generic arguments.
    ///
    /// This is a version of [`add_function`] that accepts a `FnMut` argument and accepts a debug
    /// name
    ///
    /// [`add_function`]: UserDataMethods::add_function

    fn add_function_mut_with_debug<F, A, R>(
        &mut self,
        name: impl Into<StdString>,
        debugname: &'static CStr,
        function: F,
    ) where
        F: FnMut(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod which accepts a `&T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: UserDataMethods::add_meta_function
    fn add_meta_method<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod as a function which accepts a `&mut T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: UserDataMethods::add_meta_function
    fn add_meta_method_mut<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod which accepts generic arguments.
    ///
    /// Metamethods for binary operators can be triggered if either the left or right argument to
    /// the binary operator has a metatable, so the first argument here is not necessarily a
    /// userdata of type `T`.
    fn add_meta_function<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, A) -> R + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a metamethod as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_meta_function`] that accepts a `FnMut` argument.
    ///
    /// [`add_meta_function`]: UserDataMethods::add_meta_function
    fn add_meta_function_mut<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: FnMut(&Lua, A) -> R + MaybeSend + 'static,
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
    /// If `add_meta_method` is used to set the `__index` metamethod, it will
    /// be used as a fall-back if no regular field or method are found.
    fn add_field<V>(&mut self, name: impl Into<StdString>, value: V)
    where
        V: IntoLua + 'static;

    /// Add a regular field getter as a method which accepts a `&T` as the parameter.
    ///
    /// Regular field getters are implemented by overriding the `__index` metamethod and returning
    /// the accessed field. This allows them to be used with the expected `userdata.field` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular field or method are found.
    fn add_field_method_get<M, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: Fn(&Lua, &T) -> R + MaybeSend + 'static,
        R: IntoLuaResult;

    /// Add a regular field setter as a method which accepts a `&mut T` as the first parameter.
    ///
    /// Regular field setters are implemented by overriding the `__newindex` metamethod and setting
    /// the accessed field. This allows them to be used with the expected `userdata.field = value`
    /// syntax.
    ///
    /// If `add_meta_method` is used to set the `__newindex` metamethod, the `__newindex` metamethod
    /// will be used as a fall-back if no regular field is found.
    fn add_field_method_set<M, A, R>(&mut self, name: impl Into<StdString>, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> R + MaybeSend + 'static,
        A: FromLua,
        R: IntoLuaResultMulti;

    /// Add a regular field getter as a function which accepts a generic [`AnyUserData`] of type `T`
    /// argument.
    fn add_field_function_get<F, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: Fn(&Lua, AnyUserData) -> R + MaybeSend + 'static,
        R: IntoLuaResult;

    /// Add a regular field setter as a function which accepts a generic [`AnyUserData`] of type `T`
    /// first argument.
    fn add_field_function_set<F, A, R>(&mut self, name: impl Into<StdString>, function: F)
    where
        F: FnMut(&Lua, AnyUserData, A) -> R + MaybeSend + 'static,
        A: FromLua,
        R: IntoLuaResultMulti;

    /// Add a metatable field.
    ///
    /// This will initialize the metatable field with `value` on [`UserData`] creation.
    ///
    /// # Note
    ///
    /// `mlua` will trigger an error on an attempt to define a protected metamethod,
    /// like `__gc` or `__metatable`.
    fn add_meta_field<V>(&mut self, name: impl Into<StdString>, value: V)
    where
        V: IntoLua + 'static;

    /// Add a metatable field computed from `f`.
    ///
    /// This will initialize the metatable field from `f` on [`UserData`] creation.
    ///
    /// # Note
    ///
    /// `mlua` will trigger an error on an attempt to define a protected metamethod,
    /// like `__gc` or `__metatable`.
    fn add_meta_field_with<F, R>(&mut self, name: impl Into<StdString>, f: F)
    where
        F: FnOnce(&Lua) -> R + 'static,
        R: IntoLuaResult;
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code.
///
/// Implementation of [`IntoLua`] is automatically provided, [`FromLua`] needs to be implemented
/// manually.
///
///
/// # Examples
///
/// ```
/// # use mluau::{Lua, Result, UserData};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData;
///
/// impl UserData for MyUserData {}
///
/// // `MyUserData` now implements `IntoLua`:
/// lua.globals().set("myobject", MyUserData)?;
///
/// lua.load("assert(type(myobject) == 'userdata')").exec()?;
/// # Ok(())
/// # }
/// ```
///
/// Custom fields, methods and operators can be provided by implementing `add_fields` or
/// `add_methods` (refer to [`UserDataFields`] and [`UserDataMethods`] for more information):
///
/// ```
/// # use mluau::{Lua, MetaMethod, Result, UserData, UserDataFields, UserDataMethods};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {
///     fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
///         fields.add_field_method_get("val", |_, this| Ok::<_, mluau::Error>(this.0));
///     }
///
///     fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
///         methods.add_method_mut("add", |_, mut this, value: i32| {
///             this.0 += value;
///             Ok::<_, mluau::Error>(())
///         });
///
///         methods.add_meta_method(MetaMethod::Add, |_, this, value: i32| {
///             Ok::<_, mluau::Error>(this.0 + value)
///         });
///     }
/// }
///
/// lua.globals().set("myobject", MyUserData(123))?;
///
/// lua.load(r#"
///     assert(myobject.val == 123)
///     myobject:add(7)
///     assert(myobject.val == 130)
///     assert(myobject + 10 == 140)
/// "#).exec()?;
/// # Ok(())
/// # }
/// ```
pub trait UserData: Sized {
    /// Adds custom fields specific to this userdata.
    #[allow(unused_variables)]
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    #[allow(unused_variables)]
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {}

    /// Registers this type for use in Lua.
    ///
    /// This method is responsible for calling `add_fields` and `add_methods` on the provided
    /// [`UserDataRegistry`].
    fn register(registry: &mut UserDataRegistry<Self>) {
        Self::add_fields(registry);
        Self::add_methods(registry);
    }
}

/// Handle to an internal Lua userdata for any type that implements [`UserData`].
///
/// Similar to [`std::any::Any`], this provides an interface for dynamic type checking via the
/// [`is`] and [`borrow`] methods.
///
/// # Note
///
/// This API should only be used when necessary. Implementing [`UserData`] already allows defining
/// methods which check the type and acquire a borrow behind the scenes.
///
/// [`is`]: crate::AnyUserData::is
/// [`borrow`]: crate::AnyUserData::borrow
#[derive(Clone, Debug, PartialEq)]
pub struct AnyUserData(pub(crate) ValueRef);

impl AnyUserData {
    /// Checks whether the type of this userdata is `T`.
    ///
    /// Will always return `false` for dynamic userdata.
    #[inline]
    pub fn is<T: 'static>(&self) -> bool {
        let type_id = self.type_id();
        // If the userdata is dynamic, we cannot check its type id
        if type_id.is_none() {
            return false;
        }
        // We do not use wrapped types here, rather prefer to check the "real" type of the userdata
        matches!(type_id, Some(type_id) if type_id == TypeId::of::<T>())
    }

    /// Borrow this userdata immutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a [`UserDataBorrowError`] if the userdata is already mutably borrowed.
    /// Returns a [`DataTypeMismatch`] if the userdata is not of type `T` or if it's
    /// dynamic.
    ///
    /// [`UserDataBorrowError`]: crate::Error::UserDataBorrowError
    /// [`DataTypeMismatch`]: crate::Error::UserDataTypeMismatch
    #[inline]
    pub fn borrow<T: 'static>(&self) -> Result<UserDataRef<T>> {
        let lua = self.0.lua.lock();
        unsafe { 
            let state = lua.state();
            let _sg = crate::util::StackGuard::new(state);
            lua.push_ref_at(&self.0, state);
            UserDataRef::borrow_from_stack(&lua, state, -1) 
        }
    }



    /// Borrow this userdata mutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a [`UserDataBorrowMutError`] if the userdata cannot be mutably borrowed.
    /// Returns a [`UserDataTypeMismatch`] if the userdata is not of type `T` or if it's
    /// a dynamic userdata.
    ///
    /// [`UserDataBorrowMutError`]: crate::Error::UserDataBorrowMutError
    /// [`UserDataTypeMismatch`]: crate::Error::UserDataTypeMismatch
    #[inline]
    pub fn borrow_mut<T: 'static>(&self) -> Result<UserDataRefMut<T>> {
        let lua = self.0.lua.lock();
        unsafe { 
            let state = lua.state();
            let _sg = crate::util::StackGuard::new(state);
            lua.push_ref_at(&self.0, state);
            UserDataRefMut::borrow_from_stack(&lua, state, -1) 
        }
    }



    /// Takes the value out of this userdata.
    ///
    /// Sets the special "destructed" metatable that prevents any further operations with this
    /// userdata.
    ///
    /// Keeps associated user values unchanged (they will be collected by Lua's GC).
    ///
    /// Will always return `UserDataTypeMismatch` on dynamic userdata.
    pub fn take<T: 'static>(&self) -> Result<T> {
        let lua = self.0.lua.lock();
        match lua.get_userdata_ref_type_id(&self.0)? {
            Some(type_id) if type_id == TypeId::of::<T>() => unsafe {
                let state = lua.state();
                let _sg = StackGuard::new(state);
                lua.push_ref_at(&self.0, state);
                if (*get_userdata::<UserDataStorage<T>>(state, -1)).has_exclusive_access() {
                    take_userdata::<UserDataStorage<T>>(state, -1).into_inner()
                } else {
                    Err(Error::UserDataBorrowMutError)
                }
            },
            _ => Err(Error::UserDataTypeMismatch),
        }
    }

    /// Destroys this userdata.
    ///
    /// This is similar to [`AnyUserData::take`], but it doesn't require a type.
    ///
    /// This method works for non-scoped and non-dynamic userdata only.
    pub fn destroy(&self) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();

        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            // Luau does not have __gc
            match lua.get_userdata_ref_type_id(&self.0)? {
                Some(type_id) => {
                    // Get out the destructor from extra
                    let dtor = match (&(*lua.extra())).get_userdata_dtor(type_id) {
                        Some(dtor) => dtor,
                        None => return Err(Error::UserDataTypeMismatch),
                    };

                    // Call the destructor
                    protect_lua!(state, 0, 1, |state| {
                        ffi::lua_pushcfunction(state, dtor);
                        lua.push_ref_at(&self.0, state);
                        ffi::lua_call(state, 1, 1);
                    })?;

                    if ffi::lua_isboolean(state, -1) != 0 && ffi::lua_toboolean(state, -1) != 0 {
                        return Ok(());
                    }
                    return Err(Error::UserDataBorrowMutError);
                }
                None => return Err(Error::UserDataTypeMismatch),
            }
        }
    }

    /// For a dynamic userdata, returns the inner data put into the userdata.
    ///
    /// This will return `UserDataTypeMismatch` if the userdata is not dynamic or
    /// if the dynamic userdata's metatable is not associated with the type `T`.
    #[cfg(feature = "dynamic-userdata")]
    pub fn dynamic_data<T: 'static>(&self) -> Result<&T> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = crate::util::StackGuard::new(state);
            lua.push_ref_at(&self.0, state);

            let ud_ptr = ffi::lua_topointer(state, -1);
            if !(&(*lua.extra())).is_userdata_dynamic(ud_ptr as *mut c_void) {
                return Err(Error::UserDataTypeMismatch);
            }

            let ud = get_userdata::<DynamicUserDataPtr>(state, -1);

            match (&*ud).data.downcast_ref::<T>() {
                Some(data) => Ok(data),
                None => Err(Error::UserDataTypeMismatch),
            }
        }
    }



    /// Returns a metatable of this [`AnyUserData`].
    ///
    /// Returned [`UserDataMetatable`] object wraps the original metatable and
    /// provides safe access to its methods.
    ///
    /// For `T: 'static` returned metatable is shared among all instances of type `T`.
    ///
    /// This will always return a error if used on a dynamic userdata.
    #[inline]
    pub fn metatable(&self) -> Result<UserDataMetatable> {
        self.raw_metatable().map(UserDataMetatable)
    }

    /// Returns the raw metatable of this [`AnyUserData`].
    /// without any additional checks.
    ///
    /// This is mainly useful with luau-created userdata
    /// which do not have a type id from mlua.
    ///
    /// Returns ``UserDataTypeMismatch`` if the userdata is empty or has no metatable.
    ///
    /// Safety:
    ///
    /// It is up to the user to ensure that changes made to the underlying metatable
    /// do not modify restricted mlua userdata metamethods etc. When in doubt, use
    /// ``metatable()`` instead. It is possible to cause memory unsafety by abusing
    /// ``underlying_metatable()```, for example by modifying/calling the `__gc` metamethod
    pub unsafe fn underlying_metatable(&self) -> Result<Table> {
        self.raw_underlying_metatable()
    }

    fn raw_underlying_metatable(&self) -> Result<Table> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 1)?;

            // Push the userdata onto the stack
            // Note that we cannot use `lua.push_userdata_ref_at` here
            // as that requires a type id to be present.
            lua.push_ref_at(&self.0, state);

            let res = ffi::lua_getmetatable(state, -1); // Checked that non-empty on the previous call
            if res == 0 {
                return Err(Error::UserDataTypeMismatch);
            }
            Ok(Table(lua.pop_ref()))
        }
    }

    /// Returns a raw metatable of this [`AnyUserData`].
    fn raw_metatable(&self) -> Result<Table> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            lua.push_ref_at(&self.0, state);

            // Check that userdata is registered and not destructed
            // All registered userdata types have a non-empty metatable
            let _type_id = lua.get_userdata_ref_type_id(&self.0)?;

            ffi::lua_getmetatable(state, -1);
            Ok(Table(lua.pop_ref_at(state)))
        }
    }

    /// Converts this userdata to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }

    /// Returns [`TypeId`] of this userdata if it is registered and `'static`.
    ///
    /// This method is not available for scoped userdata.
    #[inline]
    pub fn type_id(&self) -> Option<TypeId> {
        let lua = self.0.lua.lock();
        lua.get_userdata_ref_type_id(&self.0).ok().flatten()
    }

    /// Returns a type name of this `UserData` (from a metatable field).
    ///
    /// Returns ``None`` if the type name is not set, the userdata is not registered
    /// or no type metafield is set.
    pub fn type_name(&self) -> Result<Option<StdString>> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_userdata_ref_at(&self.0, state)?;
            let name_type = protect_lua!(state, 1, 1, |state| {
                ffi::luaL_getmetafield(state, -1, MetaMethod::Type.as_cstr().as_ptr())
            })?;
            match name_type {
                ffi::LUA_TSTRING => Ok(Some(String(lua.pop_ref()).to_str()?.to_owned())),
                _ => Ok(None),
            }
        }
    }

    pub(crate) fn equals(&self, other: &Self) -> Result<bool> {
        // Uses lua_rawequal() under the hood
        if self == other {
            return Ok(true);
        }

        let mt = self.raw_metatable()?;
        if mt != other.raw_metatable()? {
            return Ok(false);
        }

        if mt.contains_key("__eq")? {
            return mt.get::<Function>("__eq")?.call((self, other));
        }

        Ok(false)
    }

    #[inline]
    pub fn get<V: FromLua>(&self, key: impl IntoLua) -> Result<V> {
        // `lua_gettable` method used under the hood can work with any Lua value
        // that has `__index` metamethod
        Table(self.0.clone()).get_protected(key)
    }

    #[inline]
    pub fn set(&self, key: impl IntoLua, value: impl IntoLua) -> Result<()> {
        // `lua_settable` method used under the hood can work with any Lua value
        // that has `__newindex` metamethod
        Table(self.0.clone()).set_protected(key, value)
    }

    #[inline]
    pub fn call<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        Function(self.0.clone()).call(args)
    }

    #[inline]
    pub fn call_method<R>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        self.call_function(name, (self.clone(), args))
    }

    #[inline]
    pub fn call_function<R: FromLuaMulti>(&self, name: &str, args: impl IntoLuaMulti) -> Result<R> {
        match self.get(name)? {
            Value::Function(func) => func.call(args),
            val => {
                let msg = format!("attempt to call a {} value (function '{name}')", val.type_name());
                Err(Error::runtime(msg))
            }
        }
    }

    #[inline]
    pub fn to_string(&self) -> Result<StdString> {
        Value::UserData(self.clone()).to_string()
    }

    #[inline]
    pub fn weak_lua(&self) -> &WeakLua {
        &self.0.lua
    }
}

/// Handle to a [`AnyUserData`] metatable.
#[derive(Clone, Debug)]
pub struct UserDataMetatable(pub(crate) Table);

impl UserDataMetatable {
    /// Gets the value associated to `key` from the metatable.
    ///
    /// If no value is associated to `key`, returns the `Nil` value.
    /// Access to restricted metamethods such as `__gc` or `__metatable` will cause an error.
    pub fn get<V: FromLua>(&self, key: impl AsRef<str>) -> Result<V> {
        self.0.raw_get(MetaMethod::validate(key.as_ref())?)
    }

    /// Sets a key-value pair in the metatable.
    ///
    /// If the value is `Nil`, this will effectively remove the `key`.
    /// Access to restricted metamethods such as `__gc` or `__metatable` will cause an error.
    /// Setting `__index` or `__newindex` metamethods is also restricted because their values are
    /// cached for `mlua` internal usage.
    pub fn set(&self, key: impl AsRef<str>, value: impl IntoLua) -> Result<()> {
        let key = MetaMethod::validate(key.as_ref())?;
        // `__index` and `__newindex` cannot be changed in runtime, because values are cached
        if key == MetaMethod::Index || key == MetaMethod::NewIndex {
            return Err(Error::MetaMethodRestricted(key.to_string()));
        }
        self.0.raw_set(key, value)
    }

    /// Checks whether the metatable contains a non-nil value for `key`.
    pub fn contains(&self, key: impl AsRef<str>) -> Result<bool> {
        self.0.contains_key(MetaMethod::validate(key.as_ref())?)
    }

    /// Returns an iterator over the pairs of the metatable.
    ///
    /// The pairs are wrapped in a [`Result`], since they are lazily converted to `V` type.
    ///
    /// [`Result`]: crate::Result
    pub fn pairs<V: FromLua>(&self) -> UserDataMetatablePairs<'_, V> {
        UserDataMetatablePairs(self.0.pairs())
    }
}

/// An iterator over the pairs of a [`AnyUserData`] metatable.
///
/// It skips restricted metamethods, such as `__gc` or `__metatable`.
///
/// This struct is created by the [`UserDataMetatable::pairs`] method.
pub struct UserDataMetatablePairs<'a, V>(TablePairs<'a, StdString, V>);

impl<V> Iterator for UserDataMetatablePairs<'_, V>
where
    V: FromLua,
{
    type Item = Result<(StdString, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.0.next()? {
                Ok((key, value)) => {
                    // Skip restricted metamethods
                    if MetaMethod::validate(&key).is_ok() {
                        break Some(Ok((key, value)));
                    }
                }
                Err(e) => break Some(Err(e)),
            }
        }
    }
}


struct WrappedUserdata<F: FnOnce(&Lua) -> Result<AnyUserData>>(F);

impl AnyUserData {
    /// Wraps any Rust type, returning an opaque type that implements [`IntoLua`] trait.
    ///
    /// This function uses [`Lua::create_any_userdata`] under the hood.
    pub fn wrap<T: MaybeSend + MaybeSync + 'static>(data: T) -> impl IntoLua {
        WrappedUserdata(move |lua| lua.create_any_userdata(data))
    }


}

impl<F> IntoLua for WrappedUserdata<F>
where
    F: for<'l> FnOnce(&'l Lua) -> Result<AnyUserData>,
{
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        (self.0)(lua).map(Value::UserData)
    }
}

mod cell;
mod lock;
mod r#ref;
mod registry;
mod util;

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(AnyUserData: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(AnyUserData: Send, Sync);
}
