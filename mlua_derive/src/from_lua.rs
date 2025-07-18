use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

pub fn from_lua(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, generics, .. } = parse_macro_input!(input as DeriveInput);

    let ident_str = ident.to_string();
    let (impl_generics, ty_generics, _) = generics.split_for_impl();
    let where_clause = match &generics.where_clause {
        Some(where_clause) => quote! { #where_clause, Self: 'static + Clone },
        None => quote! { where Self: 'static + Clone },
    };

    quote! {
      impl #impl_generics ::mluau::FromLua for #ident #ty_generics #where_clause {
        #[inline]
        fn from_lua(value: ::mluau::Value, _: &::mluau::Lua) -> ::mluau::Result<Self> {
          match value {
            ::mluau::Value::UserData(ud) => Ok(ud.borrow::<Self>()?.clone()),
            _ => Err(::mluau::Error::FromLuaConversionError {
                from: value.type_name(),
                to: #ident_str.to_string(),
                message: None,
            }),
          }
        }
      }
    }
    .into()
}
