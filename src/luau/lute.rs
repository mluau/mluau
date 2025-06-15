use crate::error::{Error, Result};
use crate::state::util::get_next_spot;
use crate::state::RawLua;
use crate::util::check_stack;
use crate::{ffi, Lua, Table};
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign};

/// Flags describing the set of lute standard libraries to load.
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LuteStdLib(u32);

impl LuteStdLib {
    #[cfg(feature = "luau-lute-crypto")]
    pub const CRYPTO: LuteStdLib = LuteStdLib(1);
    pub const FS: LuteStdLib = LuteStdLib(1 << 1);
    pub const LUAU: LuteStdLib = LuteStdLib(1 << 2);
    #[cfg(feature = "luau-lute-net")]
    pub const NET: LuteStdLib = LuteStdLib(1 << 3);
    pub const PROCESS: LuteStdLib = LuteStdLib(1 << 4);
    pub const TASK: LuteStdLib = LuteStdLib(1 << 5);
    pub const VM: LuteStdLib = LuteStdLib(1 << 6);
    pub const SYSTEM: LuteStdLib = LuteStdLib(1 << 7);
    pub const TIME: LuteStdLib = LuteStdLib(1 << 8);

    /// No libraries
    pub const NONE: LuteStdLib = LuteStdLib(0);
    /// (**unsafe**) All standard libraries
    pub const ALL: LuteStdLib = LuteStdLib(u32::MAX);

    pub fn contains(self, lib: Self) -> bool {
        (self & lib).0 != 0
    }
}

impl BitAnd for LuteStdLib {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        LuteStdLib(self.0 & rhs.0)
    }
}

impl BitAndAssign for LuteStdLib {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = LuteStdLib(self.0 & rhs.0)
    }
}

impl BitOr for LuteStdLib {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        LuteStdLib(self.0 | rhs.0)
    }
}

impl BitOrAssign for LuteStdLib {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = LuteStdLib(self.0 | rhs.0)
    }
}

impl BitXor for LuteStdLib {
    type Output = Self;
    fn bitxor(self, rhs: Self) -> Self::Output {
        LuteStdLib(self.0 ^ rhs.0)
    }
}

impl BitXorAssign for LuteStdLib {
    fn bitxor_assign(&mut self, rhs: Self) {
        *self = LuteStdLib(self.0 ^ rhs.0)
    }
}

/// A handle to the lute runtime, which provides access to various standard libraries
/// and functionality within lute
#[derive(Debug, Clone)]
pub struct LuteRuntimeHandle {
    #[cfg(feature = "luau-lute-crypto")]
    pub crypto: Option<Table>,
    pub fs: Option<Table>,
    pub luau: Option<Table>,
    #[cfg(feature = "luau-lute-net")]
    pub net: Option<Table>,
    pub process: Option<Table>,
    pub task: Option<Table>,
    pub vm: Option<Table>,
    pub system: Option<Table>,
    pub time: Option<Table>,
}

impl LuteRuntimeHandle {
    pub(crate) fn new() -> Result<Self> {
        let mut handle = Self {
            #[cfg(feature = "luau-lute-crypto")]
            crypto: None,
            fs: None,
            luau: None,
            #[cfg(feature = "luau-lute-net")]
            net: None,
            process: None,
            task: None,
            vm: None,
            system: None,
            time: None,
        };

        Ok(handle)
    }
}

impl Lua {
    /// Returns if a lute runtime is loaded into the client or not
    pub fn is_lute_loaded(&self) -> Result<bool> {
        self.lock().is_lute_loaded()
    }

    /// Sets up a lute runtime on the current Lua state. This internally creates a second auxiliary
    /// VM to be created to act as the data VM
    pub fn setup_lute_runtime(&self) -> Result<()> {
        self.lock().setup_lute_runtime()
    }

    /// Destroys the lute runtime on the current Lua state. This internally destroys the auxiliary
    /// VM created to act as the data VM as well
    pub fn destroy_lute_runtime(&self) -> Result<bool> {
        self.lock().destroy_lute_runtime()
    }

    /// Loads the specified lute standard libraries into the current Lua state.
    ///
    /// This errors if the runtime is not loaded.
    pub fn load_lute_stdlib(&self, libs: LuteStdLib) -> Result<()> {
        self.lock().load_lute_stdlib(libs)
    }
}
