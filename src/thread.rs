use std::fmt;
use std::os::raw::{c_int, c_void};
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::RawLua;
use crate::traits::{FromLuaMulti, IntoLuaMulti};
use crate::types::{LuaType, TypedRef, ValueRef};

use crate::util::{check_stack, error_traceback_thread, pop_error, StackGuard};
use crate::WeakLua;

/// Continuation thread status. Can either be Ok, Yielded (rare, but can happen) or Error
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ContinuationStatus {
    Ok,
    Yielded,
    Error(c_int),
}

impl ContinuationStatus {
    #[allow(dead_code)]
    pub(crate) fn from_status(status: c_int) -> Self {
        match status {
            ffi::LUA_YIELD => Self::Yielded,
            ffi::LUA_OK => Self::Ok,
            s => Self::Error(s),
        }
    }
}

/// Status of a Lua thread (coroutine).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ThreadStatus {
    /// The thread was just created or is suspended (yielded).
    ///
    /// If a thread is in this state, it can be resumed by calling [`Thread::resume`].
    Resumable,
    /// The thread is currently running.
    Running,
    /// The thread has finished executing.
    Finished,
    /// The thread has raised a Lua error during execution.
    Error,
}

/// Internal representation of a Lua thread status.
///
/// The number in `New` and `Yielded` variants is the number of arguments pushed
/// to the thread stack.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ThreadStatusInner {
    New(c_int),
    Running,
    Yielded(c_int),
    Finished,
    Error,
}

/// Handle to an internal Lua thread (coroutine).
#[derive(Clone)]
pub struct Thread(pub(crate) ValueRef, pub(crate) *mut ffi::lua_State);

impl Thread {
    /// Returns reference to the Lua state that this thread is associated with.
    #[doc(hidden)]
    #[inline(always)]
    pub fn state(&self) -> *mut ffi::lua_State {
        self.1
    }

    /// Tries converting whatever is on the thread stack to ``R``.
    ///
    /// Useful if you know the thread has something but cannot extract it directly.
    ///
    /// # Safety
    ///
    /// Note that while this method is usually safe to call, the results returned
    /// by this method could be used to induce memory unsafety. Note that all cases
    /// of this happening, however, are bugs in mluau.
    pub fn pop_results<R>(&self) -> Result<R>
    where
        R: FromLuaMulti,
    {
        unsafe {
            let lua = self.0.lua.lock();
            let thread_state = self.state();
            let _sg = StackGuard::new(lua.state());
            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let nresults = ffi::lua_gettop(thread_state);
            R::from_specified_stack_multi(nresults, &lua, thread_state)
        }
    }

    /// Returns the thread data without removing it from the thread.
    ///
    /// Returns `None` if no data was set for the current lua thread or if the provided type
    /// does not match the stored data type.
    ///
    /// This is a Luau specific extension.
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn thread_data<T: 'static>(&self) -> Option<TypedRef<T, Self>> {
        let lua = self.0.lua.lock();
        let thread_state = self.state();
        let ptr = unsafe {
            let current = ffi::lua_getthreaddata(thread_state);
            if current.is_null() {
                return None;
            }
            crate::types::ErasedHeader::downcast_ref(current)
        };
        TypedRef::new_opt(lua.lua().clone(), ptr, self.clone())
    }

    /// Sets the thread data. The set thread data will automatically be dropped upon Luau GC
    ///
    /// Errors if thread data was already set for the current lua thread.
    ///
    /// This is a Luau specific extension.
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_thread_data<T: 'static>(&self, data: T) -> Result<()> {
        let lua = self.0.lua.lock();
        let thread_state = self.state();
        unsafe {
            let current = ffi::lua_getthreaddata(thread_state);
            if !current.is_null() {
                return Err(Error::runtime("thread data was already set for this thread"));
            }
            let raw = crate::types::ErasedHeader::into_raw(data);
            ffi::lua_setthreaddata(thread_state, raw);
            let extra = lua.extra();
            if !(*extra).have_thread_data {
                (*extra).have_thread_data = true;
                (*ffi::lua_callbacks(lua.main_state())).userthread = Some(crate::Lua::userthread_proc);
            }
        }
        Ok(())
    }

    /// Resumes execution of this thread.
    ///
    /// Equivalent to [`coroutine.resume`].
    ///
    /// Passes `args` as arguments to the thread. If the coroutine has called [`coroutine.yield`],
    /// it will return these arguments. Otherwise, the coroutine wasn't yet started, so the
    /// arguments are passed to its main function.
    ///
    /// If the thread is no longer resumable (meaning it has finished execution or encountered an
    /// error), this will return [`Error::CoroutineUnresumable`], otherwise will return `Ok` as
    /// follows:
    ///
    /// If the thread calls [`coroutine.yield`], returns the values passed to `yield`. If the thread
    /// `return`s values from its main function, returns those.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mluau::{Error, Lua, Result, Thread};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let thread: Thread = lua.load(r#"
    ///     coroutine.create(function(arg)
    ///         assert(arg == 42)
    ///         local yieldarg = coroutine.yield(123)
    ///         assert(yieldarg == 43)
    ///         return 987
    ///     end)
    /// "#).eval()?;
    ///
    /// assert_eq!(thread.resume::<u32>(42)?, 123);
    /// assert_eq!(thread.resume::<u32>(43)?, 987);
    ///
    /// // The coroutine has now returned, so `resume` will fail
    /// match thread.resume::<u32>(()) {
    ///     Err(Error::CoroutineUnresumable) => {},
    ///     unexpected => panic!("unexpected result {:?}", unexpected),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`coroutine.resume`]: https://www.lua.org/manual/5.4/manual.html#pdf-coroutine.resume
    /// [`coroutine.yield`]: https://www.lua.org/manual/5.4/manual.html#pdf-coroutine.yield
    pub fn resume<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        let mut has_yielded = false;
        let mut pushed_nargs = match self.status_inner(&lua) {
            ThreadStatusInner::New(nargs) => nargs,
            ThreadStatusInner::Yielded(nargs) => {
                has_yielded = true;
                nargs
            }
            _ => return Err(Error::CoroutineUnresumable),
        };

        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);

            if has_yielded {
                // We need to use the mainthread here as pcall over a yielded thread is not allowed
                let nargs = args.push_into_specified_stack_multi(&lua, state)?;
                if nargs > 0 {
                    check_stack(thread_state, nargs)?;
                    ffi::lua_xmove(state, thread_state, nargs);
                }
                pushed_nargs += nargs;
            } else {
                let nargs = args.push_into_specified_stack_multi(&lua, thread_state)?;
                pushed_nargs += nargs;
            }

            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let (_, nresults) = self.resume_inner(&lua, pushed_nargs)?;

            R::from_specified_stack_multi(nresults, &lua, thread_state)
        }
    }

    /// Resumes execution of this thread, immediately raising an error.
    ///
    /// This is a Luau specific extension.

    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn resume_error<R>(&self, error: impl crate::IntoLua) -> Result<R>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        let mut has_yielded = false;
        match self.status_inner(&lua) {
            ThreadStatusInner::New(_) => {}
            ThreadStatusInner::Yielded(_) => has_yielded = true,
            _ => return Err(Error::CoroutineUnresumable),
        };

        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);

            if has_yielded {
                // We need to use the mainthread here as pcall over a yielded thread is not allowed
                check_stack(state, 1)?;
                error.push_into_specified_stack(&lua, state)?;
                ffi::lua_xmove(state, thread_state, 1);
            } else {
                check_stack(thread_state, 1)?;
                error.push_into_specified_stack(&lua, thread_state)?;
            }

            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let (_, nresults) = self.resume_inner(&lua, ffi::LUA_RESUMEERROR)?;

            R::from_specified_stack_multi(nresults, &lua, thread_state)
        }
    }

    /// Resumes execution of this thread.
    ///
    /// It's similar to `resume()` but leaves `nresults` values on the thread stack.
    unsafe fn resume_inner(&self, lua: &RawLua, nargs: c_int) -> Result<(ThreadStatusInner, c_int)> {
        let state = lua.state();
        let thread_state = self.state();
        let mut nresults = 0;
        let ret = ffi::lua_resumex(thread_state, state, nargs, &mut nresults as *mut c_int);

        match ret {
            ffi::LUA_OK => Ok((ThreadStatusInner::Finished, nresults)),
            ffi::LUA_YIELD => Ok((ThreadStatusInner::Yielded(0), nresults)),
            ffi::LUA_ERRMEM => {
                // Don't call error handler for memory errors
                Err(pop_error(thread_state, ret))
            }
            _ => {
                check_stack(state, 3)?;
                protect_lua!(state, 0, 1, |state| error_traceback_thread(state, thread_state))?;
                Err(pop_error(state, ret))
            }
        }
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        match self.status_inner(&self.0.lua.lock()) {
            ThreadStatusInner::New(_) | ThreadStatusInner::Yielded(_) => ThreadStatus::Resumable,
            ThreadStatusInner::Running => ThreadStatus::Running,
            ThreadStatusInner::Finished => ThreadStatus::Finished,
            ThreadStatusInner::Error => ThreadStatus::Error,
        }
    }

    /// Gets the status of the thread (internal implementation).
    fn status_inner(&self, lua: &RawLua) -> ThreadStatusInner {
        let thread_state = self.state();
        if thread_state == lua.state() {
            // The thread is currently running
            return ThreadStatusInner::Running;
        }
        let status = unsafe { ffi::lua_status(thread_state) };
        match status {
            ffi::LUA_YIELD => {
                let top = unsafe { ffi::lua_gettop(thread_state) };
                ThreadStatusInner::Yielded(top)
            }
            ffi::LUA_OK => {
                let top = unsafe { ffi::lua_gettop(thread_state) };
                if top > 0 {
                    ThreadStatusInner::New(top - 1)
                } else {
                    ThreadStatusInner::Finished
                }
            }
            _ => ThreadStatusInner::Error,
        }
    }

    /// Resets a thread
    ///
    /// In [Lua 5.4]: cleans its call stack and closes all pending to-be-closed variables.
    /// Returns a error in case of either the original error that stopped the thread or errors
    /// in closing methods.
    ///
    /// In Luau: resets to the initial state of a newly created Lua thread.
    /// Lua threads in arbitrary states (like yielded or errored) can be reset properly.
    ///
    /// Other Lua versions can reset only new or finished threads.
    ///
    /// Sets a Lua function for the thread afterwards.
    ///
    /// [Lua 5.4]: https://www.lua.org/manual/5.4/manual.html#lua_closethread
    pub fn reset(&self, func: Function) -> Result<()> {
        let lua = self.0.lua.lock();
        let thread_state = self.state();
        unsafe {
            let status = self.status_inner(&lua);
            self.reset_inner(status)?;

            // Push function to the top of the thread stack
            lua.push_ref_at(&func.0, thread_state);

            {
                // Inherit `LUA_GLOBALSINDEX` from the main thread
                ffi::lua_xpush(lua.main_state(), thread_state, ffi::LUA_GLOBALSINDEX);
                ffi::lua_replace(thread_state, ffi::LUA_GLOBALSINDEX);
            }

            Ok(())
        }
    }

    unsafe fn reset_inner(&self, status: ThreadStatusInner) -> Result<()> {
        match status {
            ThreadStatusInner::New(_) => {
                // The thread is new, so we can just set the top to 0
                ffi::lua_settop(self.state(), 0);
                Ok(())
            }
            ThreadStatusInner::Running => Err(Error::runtime("cannot reset a running thread")),
            ThreadStatusInner::Finished => Ok(()),
            ThreadStatusInner::Yielded(_) | ThreadStatusInner::Error => {
                let thread_state = self.state();

                ffi::lua_resetthread(thread_state);

                Ok(())
            }
        }
    }

    /// Closes a thread and marks it as finished.
    ///
    /// In [Lua 5.4]: cleans its call stack and closes all pending to-be-closed variables.
    /// Returns a error in case of either the original error that stopped the thread or errors
    /// in closing methods.
    ///
    /// In Luau: resets to the initial state of a newly created Lua thread.
    /// Lua threads in arbitrary states (like yielded or errored) can be reset properly.
    ///

    pub fn close(&self) -> Result<()> {
        let lua = self.0.lua.lock();
        if self.status_inner(&lua) == ThreadStatusInner::Running {
            return Err(Error::runtime("cannot reset a running thread"));
        }

        let thread_state = self.state();
        unsafe {
            ffi::lua_resetthread(thread_state);
            Ok(())
        }
    }

    /// Enables sandbox mode on this thread.
    ///
    /// Under the hood replaces the global environment table with a new table,
    /// that performs writes locally and proxies reads to caller's global environment.
    ///
    /// This mode ideally should be used together with the global sandbox mode [`Lua::sandbox`].
    ///
    /// Please note that Luau links environment table with chunk when loading it into Lua state.
    /// Therefore you need to load chunks into a thread to link with the thread environment.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mluau::{Lua, Result};
    /// #
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let thread = lua.create_thread(lua.create_function(|lua2, ()| {
    ///     lua2.load("var = 123").exec()?;
    ///     assert_eq!(lua2.globals().get::<u32>("var")?, 123);
    ///     Ok::<_, mluau::Error>(())
    /// })?)?;
    /// thread.sandbox()?;
    /// thread.resume::<()>(())?;
    ///
    /// // The global environment should be unchanged
    /// assert_eq!(lua.globals().get::<Option<u32>>("var")?, None);
    /// # Ok(())
    /// # }
    ///

    /// ```
    pub fn sandbox(&self) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            check_stack(thread_state, 3)?;
            check_stack(state, 3)?;
            protect_lua!(state, 0, 0, |_| ffi::luaL_sandboxthread(thread_state))
        }
    }

    /// Converts this thread to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }

    /// Creates a traceback of the given thread
    pub fn traceback(&self) -> Result<StdString> {
        let lua = self.0.lua.lock();
        let thread_state = self.state();
        unsafe { lua.traceback_at(thread_state) }
    }

    #[doc(hidden)]
    pub fn weak_lua(&self) -> WeakLua {
        self.0.lua.clone()
    }
}

impl fmt::Debug for Thread {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("Thread").field(&self.0).finish()
    }
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl LuaType for Thread {
    const TYPE_ID: c_int = ffi::LUA_TTHREAD;
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_any!(Thread: Send);
}
