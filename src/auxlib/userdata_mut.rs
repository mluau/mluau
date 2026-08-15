use std::{borrow::Cow, cell::RefCell, ffi::c_int};
use crate::{AnyUserData, FromLuaMulti, IntoLua, IntoLuaResult, IntoLuaResultMulti, Lua, LuaUserDataExt, TypedUserData, USERDATA2_TAG, UserDataMethods, util::short_type_name};

pub struct LuaLock<T>(pub RefCell<T>);

impl<T> LuaLock<T> {
    pub fn new(inner: T) -> Self {
        Self(RefCell::new(inner))
    }
}

/// Method registry for [`UserDataMut`] implementors.
pub trait UserDataMethodsMut<T, const TAG: c_int = USERDATA2_TAG> {
    /// Add a regular method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;
    
    /// Add a mutable method which accepts a `&mut T` as the first parameter.
    ///
    /// Mutable methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` or `add_meta_method_mut` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method_mut<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, &mut T, A) -> R + 'static,
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
        M: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti;

    /// Add a mutable metamethod which accepts a `&mut T` as the first parameter.
    fn add_meta_method_mut<M, A, R>(&mut self, name: impl Into<&'static str>, method: M)
    where
        M: Fn(&Lua, &mut T, A) -> R + 'static,
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

/// Field registry for [`UserDataMut`] implementors.
pub trait UserDataFieldsMut<T, const TAG: c_int = USERDATA2_TAG> {
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
        M: Fn(&Lua, &T) -> R + 'static,
        R: IntoLuaResult;

    // TODO: Add field method setters
}

/// Trait for custom mutable userdata types.
///
/// See [`UserData`] for information on common userdata implementation notes.
/// 
/// Note: mutable methods will error if the userdata is already borrowed. Also note that mutable
/// userdata does incur overhead compared to normal immutable userdata. Be careful: here be dragons, 
/// this api is a *major* footgun. Users be warned.
pub trait UserDataMut<const TAG: c_int = USERDATA2_TAG>: 'static + Sized {
    /// Whether or not to use __namecall optimization. See [`UserData`] 
    /// for more info on what this means.
    const USE_NAMECALL: bool = true;

    /// Type name
    fn type_name<'a>() -> Cow<'a, str> {
        Cow::Owned(short_type_name::<Self>())
    }

    /// Adds custom fields specific to this userdata.
    #[allow(unused_variables)]
    fn add_fields<F: super::UserDataFieldsMut<Self, TAG>>(fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    #[allow(unused_variables)]
    fn add_methods<M: UserDataMethodsMut<Self, TAG>>(methods: &mut M) {}

    fn into_mut(self) -> LuaLock<Self> {
        LuaLock::new(self)
    }
}

impl<const TAG: c_int, T, M> UserDataMethodsMut<T, TAG> for M
where
    T: 'static,
    M: UserDataMethods<LuaLock<T>, TAG>, 
{
    fn add_method<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        self.add_method(name, move |lua, this: TypedUserData<LuaLock<T>, TAG>, args| {
            let inner = this.0.borrow(); 
            method(lua, &*inner, args)
        });
    }

    fn add_method_mut<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &mut T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        self.add_method(name, move |lua, this: TypedUserData<LuaLock<T>, TAG>, args| {
            let mut inner = this.0.borrow_mut(); 
            method(lua, &mut *inner, args)
        });
    }

    fn add_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F)
    where
        F: Fn(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        UserDataMethods::<LuaLock<T>, TAG>::add_function(self, name, function);
    }

    fn add_meta_method<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        self.add_meta_method(name, move |lua, this: TypedUserData<LuaLock<T>, TAG>, args| {
            let inner = this.0.borrow(); 
            method(lua, &*inner, args)
        });
    }

    fn add_meta_method_mut<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &mut T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        self.add_meta_method(name, move |lua, this: TypedUserData<LuaLock<T>, TAG>, args| {
            let mut inner = this.0.borrow_mut(); 
            method(lua, &mut *inner, args)
        });
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl Into<&'static str>, function: F)
    where
        F: Fn(&Lua, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        UserDataMethods::<LuaLock<T>, TAG>::add_meta_function(self, name, function);
    }
}

impl<const TAG: c_int, T, M> UserDataFieldsMut<T, TAG> for M
where
    T: 'static,
    M: crate::UserDataFields<LuaLock<T>, TAG>,
{
    fn add_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: crate::IntoLua + 'static,
    {
        crate::UserDataFields::<LuaLock<T>, TAG>::add_field(self, name, value);
    }

    fn add_meta_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: crate::IntoLua + 'static,
    {
        crate::UserDataFields::<LuaLock<T>, TAG>::add_meta_field(self, name, value);
    }

    fn add_field_method_get<F, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&crate::Lua, &T) -> R + 'static,
        R: crate::IntoLuaResult,
    {
        self.add_field_method_get(name, move |lua, this: crate::TypedUserData<LuaLock<T>, TAG>| {
            let inner = this.0.borrow();
            method(lua, &*inner)
        });
    }
}

impl<const TAG: c_int, T: UserDataMut<TAG>> crate::UserData<TAG> for LuaLock<T> {
    const USE_NAMECALL: bool = true;

    fn type_name<'a>() -> Cow<'a, str> {
        T::type_name()
    }

    fn add_fields<F: crate::UserDataFields<Self, TAG>>(fields: &mut F) {
        T::add_fields(fields);
    }

    fn add_methods<M: crate::UserDataMethods<Self, TAG>>(methods: &mut M) {
        T::add_methods(methods);
    }
}

pub trait LuaUserDataMutExt {
    /// Create a mutable userdata
    /// 
    /// The `T` is internally wrapped in a [`LuaLock`] for interior mutability purposes
    fn create_userdata_mut<const TAG: c_int, T: UserDataMut<TAG>>(&self, data: T) -> crate::Result<AnyUserData>;
}

impl LuaUserDataMutExt for Lua {
    fn create_userdata_mut<const TAG: c_int, T: UserDataMut<TAG>>(&self, data: T) -> crate::Result<AnyUserData> {
        self.create_userdata::<TAG, LuaLock<T>>(data.into_mut())
    }
}

pub trait UserDataMutBorrowExt {
    fn with_borrow<T: UserDataMut<TAG>, R, const TAG: c_int>(
        &self, 
        f: impl FnOnce(&T) -> R
    ) -> crate::Result<R>;

    fn with_borrow_mut<T: UserDataMut<TAG>, R, const TAG: c_int>(
        &self, 
        f: impl FnOnce(&mut T) -> R
    ) -> crate::Result<R>;
}

impl UserDataMutBorrowExt for AnyUserData {
    fn with_borrow<T: UserDataMut<TAG>, R, const TAG: c_int>(
        &self, 
        f: impl FnOnce(&T) -> R
    ) -> crate::Result<R> {
        let tref = self.borrow_with_tag::<LuaLock<T>, TAG>()
            .ok_or_else(|| crate::Error::FromLuaConversionError { 
                from: "userdata", 
                to: T::type_name().to_string(), 
                message: None 
            })?;
        
        let inner = tref.0.try_borrow()
            .map_err(|_| crate::Error::RuntimeError(format!("{} has a mutable borrow currently", T::type_name())))?;

        Ok(f(&inner))
    }

    fn with_borrow_mut<T: UserDataMut<TAG>, R, const TAG: c_int>(
        &self, 
        f: impl FnOnce(&mut T) -> R
    ) -> crate::Result<R> {
        let tref = self.borrow_with_tag::<LuaLock<T>, TAG>()
            .ok_or_else(|| crate::Error::FromLuaConversionError { 
                from: "userdata", 
                to: T::type_name().to_string(), 
                message: None 
            })?;
        
        let mut inner = tref.0.try_borrow_mut()
            .map_err(|_| crate::Error::RuntimeError(format!("{} is already mutably borrowed", T::type_name())))?;

        Ok(f(&mut inner))
    }
}