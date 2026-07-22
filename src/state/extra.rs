use std::any::TypeId;
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::ptr;
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

// Unique key to store `ExtraData` in the registry
static EXTRA_REGISTRY_KEY: u8 = 0;

const WRAPPED_FAILURE_POOL_DEFAULT_CAPACITY: usize = 64;
pub const REF_STACK_RESERVE: c_int = 3;

pub(crate) struct RefThread {
    pub(crate) ref_thread: *mut ffi::lua_State,
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
    
    #[cfg(any(feature = "luau", doc))]
    pub(crate) external_buffers: rustc_hash::FxHashSet<*mut c_void>,

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
    pub(crate) ref_thread_internal: RefThread,

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
    
    pub(super) interrupt_callback: Option<crate::types::InterruptCallback>,
    
    pub(super) gc_interrupt_callback: Option<crate::types::GcInterruptCallback>,
    
    pub(super) thread_creation_callback: Option<crate::types::ThreadCreationCallback>,
    
    pub(super) thread_collection_callback: Option<crate::types::ThreadCollectionCallback>,
    
    pub(crate) have_thread_data: bool, // It is a memory leak in this case
    
    pub(crate) running_gc: bool,
    
    pub(crate) sandboxed: bool,
    
    pub(super) compiler: Option<Compiler>,
    #[cfg(feature = "luau-jit")]
    pub(super) enable_jit: bool,

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

    
    pub(crate) mem_categories: Vec<std::ffi::CString>,
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
    pub(crate) const ERROR_TRACEBACK_IDX: c_int = 1;

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
            dyn_userdata_set: rustc_hash::FxHashSet::default(),
            #[cfg(any(feature = "luau", doc))]
            external_buffers: rustc_hash::FxHashSet::default(),
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
            
            interrupt_callback: None,
            
            gc_interrupt_callback: None,
            
            thread_creation_callback: None,
            
            thread_collection_callback: None,
            
            have_thread_data: false,
            
            sandboxed: false,
            
            compiler: None,
            #[cfg(feature = "luau-jit")]
            enable_jit: true,
            
            running_gc: false,
            #[cfg(not(feature = "lua51"))]
            yielded_values: None,
            disable_error_userdata: false,
            on_close: None,
            
            mem_categories: vec![std::ffi::CString::new("main").unwrap()],
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
        
        return (*ffi::lua_callbacks(state)).userdata as *mut _;
    }

    unsafe fn store(extra: &XRc<UnsafeCell<Self>>, state: *mut ffi::lua_State) -> Result<()> {
        
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
    
    pub(crate) unsafe fn get_userdata_dtor(&self, type_id: TypeId) -> Option<ffi::lua_CFunction> {
        self.registered_userdata_dtors.get(&type_id).copied()
    }

    #[inline(always)]
    #[cfg(feature = "dynamic-userdata")]
    pub(crate) fn is_userdata_dynamic(&self, ptr: *mut c_void) -> bool {
        self.dyn_userdata_set.contains(&ptr)
    }
}
