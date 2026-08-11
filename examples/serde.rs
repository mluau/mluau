use mluau::{Error, Lua, LuaSerdeExt, Result, Value};

fn main() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    globals.set("null", Value::None)?;
    globals.set("array_mt", lua.array_metatable())?;

    // Create a Lua table with multiple data types
    let val: Value = lua
        .load(r#"{driver = "Boris", price = null, points = setmetatable({}, array_mt)}"#)
        .eval()?;

    // Serialize the table above to JSON
    let json_str = serde_json::to_string(&val).map_err(Error::external)?;
    println!("{}", json_str);

    // Create Lua Value from JSON (or any serializable type)
    let json = serde_json::json!({
        "key": "value",
        "null": null,
        "array": [],
    });
    globals.set("json_value", lua.to_value(&json)?)?;
    lua.load(
        r#"
        assert(json_value["key"] == "value")
        assert(json_value["null"] == null)
        assert(#(json_value["array"]) == 0)
    "#,
    )
    .exec()?;

    Ok(())
}
