use std::panic::catch_unwind;

use mluau::Lua;

fn main() {
    let lua = Lua::new();
    catch_unwind(|| lua.create_table().unwrap());
}
