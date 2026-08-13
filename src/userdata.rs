use crate::{FromLua, FromLuaMulti, Function, IntoLua, IntoLuaMulti, MaybeSend, Result, Table, Value, WeakLua, state::extra::USERDATA2_TAG, types::{LuaRef, MaybeSync, ValueRef}, util::{StackGuard, assert_stack, check_stack}};

/// Handle to an internal Lua userdata
#[derive(Clone, Debug, PartialEq)]
pub struct AnyUserData(pub(crate) ValueRef);

impl AnyUserData {
    /// Returns the metatable of this [`AnyUserData`].
    pub fn metatable(&self) -> Option<Table> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);

            // Push the userdata onto the stack
            lua.push_ref_at(&self.0, state);

            let res = ffi::lua_getmetatable(state, -1); // Checked that non-empty on the previous call
            if res == 0 {
                return None;
            }
            Some(Table(lua.pop_ref()))
        }
    }

    /// Returns the metatable of this [`AnyUserData`].
    pub fn set_metatable(&self, metatable: Option<Table>) {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            lua.push_ref_at(&self.0, state);
            if let Some(metatable) = &metatable {
                lua.push_ref_at(&metatable.0, state);
            } else {
                ffi::lua_pushnil(state);
            }
            ffi::lua_setmetatable(state, -2);
        }
    }

    /// Borrow this userdata immutably if it is of type `T`.
    pub fn borrow<T: 'static + MaybeSend + MaybeSync>(&self) -> Option<LuaRef<'_, T>> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        let ptr = unsafe {
            let _sg = StackGuard::new(state);

            // Push the userdata onto the stack
            lua.push_ref_at(&self.0, state);

            let res = ffi::lua_touserdatatagged(state, -1, USERDATA2_TAG);
            crate::types::ErasedHeader::downcast_ref(res)
        };
        LuaRef::new_opt(lua.lua().clone(), ptr)
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
    pub fn to_string(&self) -> Result<String> {
        Value::UserData(self.clone()).to_string()
    }

    #[inline]
    pub fn weak_lua(&self) -> &WeakLua {
        &self.0.lua
    }

    pub(crate) fn equals(&self, other: &Self) -> Result<bool> {
        // Uses lua_rawequal() under the hood
        if self == other {
            return Ok(true);
        }

        let mt = self.metatable();
        if mt != other.metatable() {
            return Ok(false);
        }

        if let Some(mt) = mt {
            if mt.contains_key("__eq")? {
                return mt.get::<Function>("__eq")?.call((self, other));
            }
        }

        Ok(false)
    }

    /// Returns a type name of this `UserData` (from a metatable field).
    ///
    /// Returns ``None`` if the type name is not set
    pub fn type_name(&self) -> Result<Option<String>> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 3)?;

            lua.push_ref_at(&self.0, state);
            let name_type = protect_lua!(state, 1, 1, |state| {
                ffi::luaL_getmetafield(state, -1, c"__type".as_ptr())
            })?;
            match name_type {
                ffi::LUA_TSTRING => Ok(Some(crate::String(lua.pop_ref()).to_str()?.to_owned())),
                _ => Ok(None),
            }
        }
    }
}