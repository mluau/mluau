use std::any::TypeId;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::Mutex;
use rustc_hash::FxHashMap;
#[cfg(feature = "dynamic-userdata")]
use rustc_hash::FxHashSet;

use crate::error::Result;
use crate::state::RawLua;
use crate::stdlib::StdLib;
use crate::types::{AppData, ReentrantMutex, XRc};

use crate::userdata::RawUserDataRegistry;
use crate::util::{get_internal_metatable, push_internal_userdata, TypeKey, WrappedFailure};

#[cfg(any(feature = "luau", doc))]
use crate::chunk::Compiler;
use crate::MultiValue;

use super::{Lua, WeakLua};

#[cfg(feature = "luau-lute")]
use crate::luau::lute::{LuteChildVmType, LuteRuntimeHandle};

// Unique key to store `ExtraData` in the registry
static EXTRA_REGISTRY_KEY: u8 = 0;

const WRAPPED_FAILURE_POOL_DEFAULT_CAPACITY: usize = 64;
pub const REF_STACK_RESERVE: c_int = 3;

pub(crate) struct RefThread {
    pub(super) ref_thread: *mut ffi::lua_State,
    pub(super) stack_size: c_int,
    pub(super) stack_top: c_int,
    pub(super) free: Vec<c_int>,
}

impl RefThread {
    #[inline(always)]
    pub(crate) unsafe fn new(state: *mut ffi::lua_State) -> Self {
        // Create ref stack thread and place it in the registry to prevent it
        // from being garbage collected.
        let ref_thread = mlua_expect!(
            protect_lua!(state, 0, 0, |state| {
                let thread = ffi::lua_newthread(state);
                ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);
                thread
            }),
            "Error while creating ref thread",
        );

        // Store `error_traceback` function on the ref stack
        #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
        {
            ffi::lua_pushcfunction(ref_thread, crate::util::error_traceback);
            assert_eq!(ffi::lua_gettop(ref_thread), ExtraData::ERROR_TRACEBACK_IDX);
        }

        RefThread {
            ref_thread,
            // We need some reserved stack space to move values in and out of the ref stack.
            stack_size: ffi::LUA_MINSTACK - REF_STACK_RESERVE,
            stack_top: ffi::lua_gettop(ref_thread),
            free: Vec::new(),
        }
    }
}

/// Data associated with the Lua state.
pub(crate) struct ExtraData {
    pub(super) lua: MaybeUninit<Lua>,
    pub(super) weak: MaybeUninit<WeakLua>,
    pub(super) owned: bool,

    pub(super) pending_userdata_reg: FxHashMap<TypeId, RawUserDataRegistry>,
    pub(super) registered_userdata_dtors: FxHashMap<TypeId, ffi::lua_CFunction>,
    pub(super) registered_userdata_t: FxHashMap<TypeId, c_int>,
    pub(super) registered_userdata_mt: FxHashMap<*const c_void, Option<TypeId>>,
    pub(super) last_checked_userdata_mt: (*const c_void, Option<TypeId>),

    #[cfg(feature = "dynamic-userdata")]
    pub(crate) dyn_userdata_set: FxHashSet<*mut c_void>,

    // When Lua instance dropped, setting `None` would prevent collecting `RegistryKey`s
    pub(super) registry_unref_list: Arc<Mutex<Option<Vec<c_int>>>>,

    // Containers to store arbitrary data (extensions)
    pub(super) app_data: AppData,
    pub(super) app_data_priv: AppData,

    pub(super) safe: bool,
    pub(super) libs: StdLib,
    // Used in module mode
    pub(super) skip_memory_check: bool,

    // Auxiliary threads to store references
    pub(super) ref_thread: Vec<RefThread>,
    // Special auxiliary thread for mlua internal use
    pub(super) ref_thread_internal: RefThread,

    // Pool of `WrappedFailure` enums in the ref thread (as userdata)
    pub(super) wrapped_failure_pool: Vec<c_int>,
    pub(super) wrapped_failure_top: usize,

    // Address of `WrappedFailure` metatable
    pub(super) wrapped_failure_mt_ptr: *const c_void,

    #[cfg(not(feature = "luau"))]
    pub(super) hook_callback: Option<crate::types::HookCallback>,
    #[cfg(not(feature = "luau"))]
    pub(super) hook_triggers: crate::debug::HookTriggers,
    #[cfg(feature = "lua54")]
    pub(super) warn_callback: Option<crate::types::WarnCallback>,
    #[cfg(feature = "luau")]
    pub(super) interrupt_callback: Option<crate::types::InterruptCallback>,
    #[cfg(feature = "luau")]
    pub(super) gc_interrupt_callback: Option<crate::types::GcInterruptCallback>,
    #[cfg(feature = "luau")]
    pub(super) thread_creation_callback: Option<crate::types::ThreadCreationCallback>,
    #[cfg(feature = "luau")]
    pub(super) thread_collection_callback: Option<crate::types::ThreadCollectionCallback>,

    #[cfg(feature = "luau")]
    pub(crate) running_gc: bool,
    #[cfg(feature = "luau")]
    pub(crate) sandboxed: bool,
    #[cfg(feature = "luau")]
    pub(super) compiler: Option<Compiler>,
    #[cfg(feature = "luau-jit")]
    pub(super) enable_jit: bool,

    #[cfg(feature = "luau-lute")]
    pub(crate) lute_handle: Option<LuteRuntimeHandle>,

    #[cfg(all(feature = "luau-lute", feature = "send"))]
    pub(crate) lute_runtimeinitter:
        Option<Box<dyn Fn(&Lua, &Lua, LuteChildVmType) -> Result<()> + Send + Sync + 'static>>,
    #[cfg(all(feature = "luau-lute", not(feature = "send")))]
    pub(crate) lute_runtimeinitter: Option<Box<dyn Fn(&Lua, &Lua, LuteChildVmType) -> Result<()> + 'static>>,

    // Child lua VM's may not be dropped from mluau
    #[cfg(feature = "luau-lute")]
    pub(crate) no_drop: bool,

    // Disable error userdata in mlua errors
    pub disable_error_userdata: bool,
    // Optional fallback lua string

    // Values currently being yielded from Lua.yield()
    #[cfg(not(feature = "lua51"))]
    pub(super) yielded_values: Option<MultiValue>,

    // Callback called when lua VM is about to be closed
    #[cfg(feature = "send")]
    pub(super) on_close: Option<Box<dyn Fn() + Send + 'static>>,
    #[cfg(not(feature = "send"))]
    pub(super) on_close: Option<Box<dyn Fn() + 'static>>,
}

impl Drop for ExtraData {
    fn drop(&mut self) {
        unsafe {
            if !self.owned {
                self.lua.assume_init_drop();
            }

            self.weak.assume_init_drop();
        }
        *self.registry_unref_list.lock() = None;
    }
}

static EXTRA_TYPE_KEY: u8 = 0;

impl TypeKey for XRc<UnsafeCell<ExtraData>> {
    #[inline(always)]
    fn type_key() -> *const c_void {
        &EXTRA_TYPE_KEY as *const u8 as *const c_void
    }
}

impl ExtraData {
    // Index of `error_traceback` function in auxiliary thread stack
    #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
    pub(super) const ERROR_TRACEBACK_IDX: c_int = 1;

    pub(super) unsafe fn init(state: *mut ffi::lua_State, owned: bool) -> XRc<UnsafeCell<Self>> {
        let wrapped_failure_mt_ptr = {
            get_internal_metatable::<WrappedFailure>(state);
            let ptr = ffi::lua_topointer(state, -1);
            ffi::lua_pop(state, 1);
            ptr
        };

        #[allow(clippy::arc_with_non_send_sync)]
        let extra = XRc::new(UnsafeCell::new(ExtraData {
            lua: MaybeUninit::uninit(),
            weak: MaybeUninit::uninit(),
            owned,
            pending_userdata_reg: FxHashMap::default(),
            registered_userdata_dtors: FxHashMap::default(),
            registered_userdata_t: FxHashMap::default(),
            registered_userdata_mt: FxHashMap::default(),
            last_checked_userdata_mt: (ptr::null(), None),
            #[cfg(feature = "dynamic-userdata")]
            dyn_userdata_set: FxHashSet::default(),
            registry_unref_list: Arc::new(Mutex::new(Some(Vec::new()))),
            app_data: AppData::default(),
            app_data_priv: AppData::default(),
            safe: false,
            libs: StdLib::NONE,
            skip_memory_check: false,
            ref_thread: vec![RefThread::new(state)],
            ref_thread_internal: RefThread::new(state),
            wrapped_failure_pool: Vec::with_capacity(WRAPPED_FAILURE_POOL_DEFAULT_CAPACITY),
            wrapped_failure_top: 0,
            wrapped_failure_mt_ptr,
            #[cfg(not(feature = "luau"))]
            hook_callback: None,
            #[cfg(not(feature = "luau"))]
            hook_triggers: Default::default(),
            #[cfg(feature = "lua54")]
            warn_callback: None,
            #[cfg(feature = "luau")]
            interrupt_callback: None,
            #[cfg(feature = "luau")]
            gc_interrupt_callback: None,
            #[cfg(feature = "luau")]
            thread_creation_callback: None,
            #[cfg(feature = "luau")]
            thread_collection_callback: None,
            #[cfg(feature = "luau")]
            sandboxed: false,
            #[cfg(feature = "luau")]
            compiler: None,
            #[cfg(feature = "luau-jit")]
            enable_jit: true,
            #[cfg(feature = "luau")]
            running_gc: false,
            #[cfg(feature = "luau-lute")]
            lute_handle: None,
            #[cfg(feature = "luau-lute")]
            lute_runtimeinitter: None,
            #[cfg(feature = "luau-lute")]
            no_drop: false,
            #[cfg(not(feature = "lua51"))]
            yielded_values: None,
            disable_error_userdata: false,
            on_close: None,
        }));

        // Store it in the registry
        mlua_expect!(Self::store(&extra, state), "Error while storing extra data");

        extra
    }

    pub(super) unsafe fn set_lua(&mut self, raw: &XRc<ReentrantMutex<RawLua>>) {
        self.lua.write(Lua {
            raw: XRc::clone(raw),
            collect_garbage: false,
        });
        self.weak.write(WeakLua(XRc::downgrade(raw)));
    }

    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        #[cfg(feature = "luau")]
        if cfg!(not(feature = "module")) {
            // In the main app we can use `lua_callbacks` to access ExtraData
            return (*ffi::lua_callbacks(state)).userdata as *mut _;
        }

        let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
        if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, extra_key) != ffi::LUA_TUSERDATA {
            // `ExtraData` can be null only when Lua state is foreign.
            // This case in used in `Lua::try_from_ptr()`.
            ffi::lua_pop(state, 1);
            return ptr::null_mut();
        }
        let extra_ptr = ffi::lua_touserdata(state, -1) as *mut Rc<UnsafeCell<ExtraData>>;
        ffi::lua_pop(state, 1);
        (*extra_ptr).get()
    }

    unsafe fn store(extra: &XRc<UnsafeCell<Self>>, state: *mut ffi::lua_State) -> Result<()> {
        #[cfg(feature = "luau")]
        if cfg!(not(feature = "module")) {
            (*ffi::lua_callbacks(state)).userdata = extra.get() as *mut _;
            return Ok(());
        }

        push_internal_userdata(state, XRc::clone(extra), true)?;
        protect_lua!(state, 1, 0, fn(state) {
            let extra_key = &EXTRA_REGISTRY_KEY as *const u8 as *const c_void;
            ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, extra_key);
        })
    }

    #[inline(always)]
    pub(super) unsafe fn lua(&self) -> &Lua {
        self.lua.assume_init_ref()
    }

    #[inline(always)]
    pub(crate) unsafe fn raw_lua(&self) -> &RawLua {
        &*self.lua.assume_init_ref().raw.data_ptr()
    }

    #[inline(always)]
    pub(super) unsafe fn weak(&self) -> &WeakLua {
        self.weak.assume_init_ref()
    }

    #[inline(always)]
    #[cfg(feature = "luau")]
    pub(crate) unsafe fn get_userdata_dtor(&self, type_id: TypeId) -> Option<ffi::lua_CFunction> {
        self.registered_userdata_dtors.get(&type_id).copied()
    }

    #[inline(always)]
    #[cfg(feature = "dynamic-userdata")]
    pub(crate) fn is_userdata_dynamic(&self, ptr: *mut c_void) -> bool {
        self.dyn_userdata_set.contains(&ptr)
    }
}
