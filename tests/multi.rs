use mluau::{Integer, Lua, MultiValue, Result, String, Value, Variadic};


#[test]
fn test_multivalue() {
    let mut multi = MultiValue::with_capacity(3);
    multi.push_back(Value::Integer(1));
    multi.push_back(Value::Integer(2));
    multi.push_front(Value::Integer(3));
    assert_eq!(multi.iter().filter_map(|v| v.as_integer()).sum::<Integer>(), 6);

    let vec = multi.into_vec();
    assert_eq!(&vec, &[Value::Integer(3), Value::Integer(1), Value::Integer(2)]);
    let _multi2 = MultiValue::from_vec(vec);
}

#[test]
fn test_multivalue_by_ref() -> Result<()> {
    let lua = Lua::new();
    let multi = MultiValue::from_vec(vec![
        Value::Integer(3),
        Value::String(lua.create_string("hello")?),
        Value::Boolean(true),
    ]);

    let f = lua.create_function(|_, (i, s, b): (i32, String, bool)| {
        assert_eq!(i, 3);
        assert_eq!(s.to_str()?, "hello");
        assert_eq!(b, true);
        Ok(())
    })?;
    f.call::<()>(&multi)?;

    Ok(())
}

#[test]
fn test_variadic() {
    let mut var = Variadic::with_capacity(3);
    var.extend_from_slice(&[1, 2, 3]);
    assert_eq!(var.iter().sum::<u32>(), 6);

    let vec = Vec::<u32>::from(var);
    assert_eq!(&vec, &[1, 2, 3]);
    let var2 = Variadic::from(vec);
    assert_eq!(var2.as_slice(), &[1, 2, 3]);
}
