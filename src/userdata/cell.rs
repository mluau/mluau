use std::cell::UnsafeCell;

use crate::error::{Error, Result};
use crate::types::XRc;

use super::lock::{RawLock, UserDataLock};
use super::r#ref::{UserDataRef, UserDataRefMut};

// A struct for storing userdata values.
// It's stored inside a Lua VM and protected by the outer `ReentrantMutex`.
pub(crate) struct UserDataStorage<T>(XRc<UserDataCell<T>>);

impl<T> Clone for UserDataStorage<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(XRc::clone(&self.0))
    }
}

impl<T> UserDataStorage<T> {
    #[inline(always)]
    pub(super) fn try_borrow_scoped<R>(&self, f: impl FnOnce(&T) -> R) -> Result<R> {
        // Shared (read) lock is always correct for in-place borrows:
        // - this method is called internally while the Lua mutex is held, ensuring exclusive Lua-level
        //   access per call frame
        // - with `send` feature, all owned userdata satisfies `T: Sync`, so simultaneous shared references
        //   from multiple threads are sound
        // - without `send` feature, single-threaded execution makes shared lock safe for any `T`
        let _guard = (self.0.raw_lock.try_lock_shared_guarded()).map_err(|_| Error::UserDataBorrowError)?;
        Ok(f(unsafe { &*self.0.value.get() }))
    }

    // Mutably borrows the wrapped value in-place.
    #[inline(always)]
    pub(crate) fn try_borrow_scoped_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R> {
        let _guard =
            (self.0.raw_lock.try_lock_exclusive_guarded()).map_err(|_| Error::UserDataBorrowMutError)?;
        Ok(f(unsafe { &mut *self.0.value.get() }))
    }

    // Immutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned(&self) -> Result<UserDataRef<T>> {
        UserDataRef::try_from(self.clone())
    }

    // Mutably borrows the wrapped value and returns an owned reference.
    #[inline(always)]
    pub(crate) fn try_borrow_owned_mut(&self) -> Result<UserDataRefMut<T>> {
        UserDataRefMut::try_from(self.clone())
    }

    // Returns the wrapped value.
    //
    // This method checks that we have exclusive access to the value.
    pub(crate) fn into_inner(self) -> Result<T> {
        if !self.0.raw_lock.try_lock_exclusive() {
            return Err(Error::UserDataBorrowMutError);
        }
        Ok(XRc::into_inner(self.0).unwrap().value.into_inner())
    }

    #[inline(always)]
    pub(crate) fn is_safe_to_destroy(&self) -> bool {
        XRc::strong_count(&self.0) > 1 || !self.0.raw_lock.is_locked()
    }

    #[inline(always)]
    pub(crate) fn has_exclusive_access(&self) -> bool {
        !self.0.raw_lock.is_locked()
    }

    #[inline(always)]
    pub(super) fn raw_lock(&self) -> &RawLock {
        &self.0.raw_lock
    }

    #[inline(always)]
    pub(super) fn as_ptr(&self) -> *mut T {
        self.0.value.get()
    }
}

/// A type that provides interior mutability for a userdata value (thread-safe).
pub(crate) struct UserDataCell<T> {
    pub(crate) raw_lock: RawLock,
    pub(crate) value: UnsafeCell<T>,
}

#[cfg(feature = "send")]
unsafe impl<T: Send> Send for UserDataCell<T> {}
#[cfg(feature = "send")]
unsafe impl<T: Send> Sync for UserDataCell<T> {}

impl<T> UserDataCell<T> {
    #[inline(always)]
    fn new(value: T) -> Self {
        UserDataCell {
            raw_lock: RawLock::INIT,
            value: UnsafeCell::new(value),
        }
    }
}

impl<T: 'static> UserDataStorage<T> {
    #[inline(always)]
    pub(crate) fn new(data: T) -> Self {
        Self(XRc::new(UserDataCell::new(data)))
    }
}
