use std::borrow::Cow;
use std::os::raw::c_int;

use ffi::{lua_Debug, lua_State};

use crate::function::Function;
use crate::state::RawLua;
use crate::util::{assert_stack, linenumber_to_usize, ptr_to_lossy_str, ptr_to_str, StackGuard};
 
/// Contains information about currently executing Lua code.
///
/// You may call the methods on this structure to retrieve information about the Lua code executing
/// at the specific level. Further information can be found in the Lua [documentation].
///
/// [documentation]: https://www.lua.org/manual/5.4/manual.html#lua_Debug
pub struct Debug<'a> {
    state: *mut lua_State,
    lua: &'a RawLua,
    #[cfg_attr(not(feature = "luau"), allow(unused))]
    level: c_int,
    ar: *mut lua_Debug,
}

impl<'a> Debug<'a> {
    pub(crate) fn new(lua: &'a RawLua, level: c_int, ar: *mut lua_Debug) -> Self {
        Debug {
            state: lua.state(),
            lua,
            ar,
            level,
        }
    }

    /// Returns the function that is running at the given level.
    ///
    /// Corresponds to the `f` "what" mask.
    pub fn function(&self) -> Function {
        {
            let extra = self.lua.extra();
            if unsafe { (*extra).running_gc } {
                panic!("Cannot get function while running GC");
            }
        }

        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("f"), self.ar) != 0,
                "lua_getinfo failed with `f`"
            );

            Function(self.lua.pop_ref_at(self.state))
        }
    }

    /// Corresponds to the `n` "what" mask.
    pub fn names(&self) -> DebugNames<'_> {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("n"), self.ar) != 0,
                "lua_getinfo failed with `n`"
            );

            DebugNames {
                name: ptr_to_lossy_str((*self.ar).name),
                name_what: None,
            }
        }
    }

    /// Corresponds to the `S` "what" mask.
    pub fn source(&self) -> DebugSource<'_> {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("s"), self.ar) != 0,
                "lua_getinfo failed with `s`"
            );

            DebugSource {
                source: ptr_to_lossy_str((*self.ar).source),
                short_src: ptr_to_lossy_str((*self.ar).short_src),
                line_defined: linenumber_to_usize((*self.ar).linedefined),
                last_line_defined: None,
                what: ptr_to_str((*self.ar).what).unwrap_or("main"),
            }
        }
    }

    #[doc(hidden)]
    #[deprecated(note = "Use `current_line` instead")]
    pub fn curr_line(&self) -> i32 {
        self.current_line().map(|n| n as i32).unwrap_or(-1)
    }

    /// Corresponds to the `l` "what" mask. Returns the current line.
    pub fn current_line(&self) -> Option<usize> {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("l"), self.ar) != 0,
                "lua_getinfo failed with `l`"
            );

            linenumber_to_usize((*self.ar).currentline)
        }
    }

    /// Corresponds to the `au` "what" mask.
    pub fn stack(&self) -> DebugStack {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("au"), self.ar) != 0,
                "lua_getinfo failed with `au`"
            );

            let stack = DebugStack {
                num_ups: (*self.ar).nupvals,
                num_params: (*self.ar).nparams,
                is_vararg: (*self.ar).isvararg != 0,
            };
            stack
        }
    }
}

/// Represents a specific event that triggered the hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugEvent {
    Call,
    Ret,
    TailCall,
    Line,
    Count,
    Unknown(c_int),
}

#[derive(Clone, Debug)]
pub struct DebugNames<'a> {
    /// A (reasonable) name of the function (`None` if the name cannot be found).
    pub name: Option<Cow<'a, str>>,
    /// Explains the `name` field (can be `global`/`local`/`method`/`field`/`upvalue`/etc).
    ///
    /// Always `None` for Luau.
    pub name_what: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct DebugSource<'a> {
    /// Source of the chunk that created the function.
    pub source: Option<Cow<'a, str>>,
    /// A "printable" version of `source`, to be used in error messages.
    pub short_src: Option<Cow<'a, str>>,
    /// The line number where the definition of the function starts.
    pub line_defined: Option<usize>,
    /// The line number where the definition of the function ends (not set by Luau).
    pub last_line_defined: Option<usize>,
    /// A string `Lua` if the function is a Lua function, `C` if it is a C function, `main` if it is
    /// the main part of a chunk.
    pub what: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct DebugStack {
    /// Number of upvalues.
    pub num_ups: u8,
    /// Number of parameters.


    pub num_params: u8,
    /// Whether the function is a vararg function.


    pub is_vararg: bool,
}
