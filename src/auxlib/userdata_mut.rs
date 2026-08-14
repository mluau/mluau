use std::cell::RefCell;
use crate::{FromLuaMulti, IntoLua, IntoLuaResult, IntoLuaResultMulti, Lua, TypedUserData, UserDataMethods};

pub struct LuaLock<T>(pub RefCell<T>);

impl<T> LuaLock<T> {
    pub fn new(inner: T) -> Self {
        Self(RefCell::new(inner))
    }
}

/// Method registry for [`UserDataMut`] implementors.
pub trait UserDataMethodsMut<T> {
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
pub trait UserDataFieldsMut<T> {
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
pub trait UserDataMut: 'static + Sized {
    /// Whether or not to use __namecall optimization. See [`UserData`] 
    /// for more info on what this means.
    const USE_NAMECALL: bool = true;

    /// Type name
    fn type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Adds custom fields specific to this userdata.
    #[allow(unused_variables)]
    fn add_fields<F: super::UserDataFieldsMut<Self>>(fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    #[allow(unused_variables)]
    fn add_methods<M: UserDataMethodsMut<Self>>(methods: &mut M) {}

    fn into_mut(self) -> LuaLock<Self> {
        LuaLock::new(self)
    }
}

impl<T, M> UserDataMethodsMut<T> for M
where
    T: 'static,
    M: UserDataMethods<LuaLock<T>>, 
{
    fn add_method<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti 
    {
        self.add_method(name, move |lua, this: TypedUserData<LuaLock<T>>, args| {
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
        self.add_method(name, move |lua, this: TypedUserData<LuaLock<T>>, args| {
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
        UserDataMethods::<LuaLock<T>>::add_function(self, name, function);
    }

    fn add_meta_method<F, A, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&Lua, &T, A) -> R + 'static,
        A: FromLuaMulti,
        R: IntoLuaResultMulti,
    {
        self.add_meta_method(name, move |lua, this: TypedUserData<LuaLock<T>>, args| {
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
        self.add_meta_method(name, move |lua, this: TypedUserData<LuaLock<T>>, args| {
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
        UserDataMethods::<LuaLock<T>>::add_meta_function(self, name, function);
    }
}

impl<T, M> UserDataFieldsMut<T> for M
where
    T: 'static,
    M: crate::UserDataFields<LuaLock<T>>,
{
    fn add_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: crate::IntoLua + 'static,
    {
        crate::UserDataFields::<LuaLock<T>>::add_field(self, name, value);
    }

    fn add_meta_field<V>(&mut self, name: impl Into<&'static str>, value: V)
    where
        V: crate::IntoLua + 'static,
    {
        crate::UserDataFields::<LuaLock<T>>::add_meta_field(self, name, value);
    }

    fn add_field_method_get<F, R>(&mut self, name: impl Into<&'static str>, method: F)
    where
        F: Fn(&crate::Lua, &T) -> R + 'static,
        R: crate::IntoLuaResult,
    {
        self.add_field_method_get(name, move |lua, this: crate::TypedUserData<LuaLock<T>>| {
            let inner = this.0.borrow();
            method(lua, &*inner)
        });
    }
}

impl<T: UserDataMut> crate::UserData for LuaLock<T> {
    const USE_NAMECALL: bool = true;

    fn type_name() -> &'static str {
        T::type_name()
    }

    fn add_fields<F: crate::UserDataFields<Self>>(fields: &mut F) {
        T::add_fields(fields);
    }

    fn add_methods<M: crate::UserDataMethods<Self>>(methods: &mut M) {
        T::add_methods(methods);
    }
}
