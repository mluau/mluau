use std::alloc::{self, Layout};
use std::os::raw::c_void;
use std::ptr;

/// A trait for custom memory allocators used by the Lua VM.
pub trait LuaAllocator: Send + 'static {
    // Same as allocator trait
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8;
    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout);
    unsafe fn realloc(&mut self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8; // layout=old layout, new_size is new size to grow/shrink to
    
    // Memory tracking+limiting
    fn used_memory(&self) -> isize { 0 }
    fn memory_limit(&self) -> isize { 0 }
    fn set_memory_limit(&mut self, _size: isize) -> isize { 0 } // optional
}

/// The default memory allocator using Rust's `std::alloc`.
pub struct DefaultAllocator {
    pub used_memory: isize,
    pub memory_limit: isize,
}

impl Default for DefaultAllocator {
    fn default() -> Self {
        Self {
            used_memory: 0,
            memory_limit: 0,
        }
    }
}

impl LuaAllocator for DefaultAllocator {
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size() as isize;
        let limit = self.memory_limit;
        let used = self.used_memory;
        
        if limit > 0 && used + size > limit {
            return ptr::null_mut();
        }
        
        let p = alloc::alloc(layout);
        if !p.is_null() {
            self.used_memory += size;
        }
        p
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        alloc::dealloc(ptr, layout);
        self.used_memory -= layout.size() as isize;
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let diff = new_size as isize - layout.size() as isize;
        let limit = self.memory_limit;
        let used = self.used_memory;
        
        if limit > 0 && used + diff > limit {
            return ptr::null_mut();
        }
        
        let p = alloc::realloc(ptr, layout, new_size);
        if !p.is_null() {
            self.used_memory += diff;
        }
        p
    }

    fn used_memory(&self) -> isize {
        self.used_memory
    }
    fn memory_limit(&self) -> isize {
        self.memory_limit
    }
    fn set_memory_limit(&mut self, size: isize) -> isize {
        let old = self.memory_limit;
        self.memory_limit = size;
        old
    }
}

pub(crate) static ALLOCATOR: ffi::lua_Alloc = allocator;

#[repr(C)]
pub(crate) struct MemoryState {
    pub(crate) allocator: Box<dyn LuaAllocator>,
    // Indicates that the memory limit was reached on the last allocation.
    limit_reached: bool,
}

impl MemoryState {
    pub(crate) fn new(allocator: Box<dyn LuaAllocator>) -> Self {
        Self {
            allocator,
            limit_reached: false,
        }
    }

    #[inline]
    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        let mut mem_state = ptr::null_mut();
        ffi::lua_getallocf(state, &mut mem_state);
        mlua_assert!(!mem_state.is_null(), "Luau state has no allocator userdata");
        mem_state as *mut MemoryState
    }


    #[inline]
    pub(crate) fn used_memory(&self) -> isize {
        self.allocator.used_memory()
    }

    #[inline]
    pub(crate) fn memory_limit(&self) -> isize {
        self.allocator.memory_limit()
    }

    #[inline]
    pub(crate) fn set_memory_limit(&mut self, size: isize) -> isize {
        self.allocator.set_memory_limit(size)
    }

    // Returns `true` if the memory limit was reached on the last memory operation
    #[inline]
    pub(crate) unsafe fn limit_reached(state: *mut ffi::lua_State) -> bool {
        (*Self::get(state)).limit_reached
    }
}

unsafe extern "C" fn allocator(
    extra: *mut c_void,
    ptr: *mut c_void,
    osize: usize,
    nsize: usize,
) -> *mut c_void {
    let mem_state = &mut *(extra as *mut MemoryState);

    // Reset the flag
    mem_state.limit_reached = false;

    if nsize == 0 {
        // Free memory
        if !ptr.is_null() {
            let layout = Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
            mem_state.allocator.dealloc(ptr as *mut u8, layout);
        }
        return ptr::null_mut();
    }

    // Do not allocate more than isize::MAX
    if nsize > isize::MAX as usize {
        return ptr::null_mut();
    }

    let new_ptr = if ptr.is_null() {
        // Allocate new memory
        let new_layout = match Layout::from_size_align(nsize, ffi::SYS_MIN_ALIGN) {
            Ok(layout) => layout,
            Err(_) => return ptr::null_mut(),
        };
        let p = mem_state.allocator.alloc(new_layout) as *mut c_void;
        if p.is_null() {
            mem_state.limit_reached = true;
        }
        p
    } else {
        // Reallocate memory
        let old_layout = Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
        let p = mem_state.allocator.realloc(ptr as *mut u8, old_layout, nsize) as *mut c_void;
        if p.is_null() {
            mem_state.limit_reached = true;
        }
        p
    };
        
    new_ptr
}


#[cfg(feature = "bumpalo")]
pub struct BumpAllocator {
    bump: bumpalo::Bump,
}

#[cfg(feature = "bumpalo")]
impl BumpAllocator {
    pub fn new() -> Self {
        Self {
            bump: bumpalo::Bump::new(),
        }
    }
}

#[cfg(feature = "bumpalo")]
impl Default for BumpAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "bumpalo")]
impl LuaAllocator for BumpAllocator {
    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        match self.bump.try_alloc_layout(layout) {
            Ok(ptr) => ptr.as_ptr(),
            Err(_) => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&mut self, _ptr: *mut u8, _layout: Layout) {
        // no-op as this is a bump allocator
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // bumpalo doesn't support realloc, so we allocate a new block and copy
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };
        match self.bump.try_alloc_layout(new_layout) {
            Ok(new_ptr) => {
                let copy_size = std::cmp::min(layout.size(), new_size);
                ptr::copy_nonoverlapping(ptr, new_ptr.as_ptr(), copy_size);
                new_ptr.as_ptr()
            }
            Err(_) => ptr::null_mut(),
        }
    }

    fn used_memory(&self) -> isize {
        self.bump.allocated_bytes() as isize
    }
    fn memory_limit(&self) -> isize { 
        self.bump.allocation_limit().unwrap_or_default() as isize
    }
    fn set_memory_limit(&mut self, size: isize) -> isize { 
        assert!(size >= 0);
        let curr = self.memory_limit();
        self.bump.set_allocation_limit(Some(size as usize));
        curr
    }
}
