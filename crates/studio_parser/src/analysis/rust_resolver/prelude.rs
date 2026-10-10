//! The std prelude (per edition) and the primitive types, as external paths.

use super::Ns;

/// The canonical std path of a prelude name in one name space (`TryFrom`, `TryInto` and `FromIterator` joined the
/// prelude in edition 2021).
pub(crate) fn std_prelude(name: &str, ns: Ns, edition: u16) -> Option<&'static str> {
    if edition < 2021 && matches!(name, "TryFrom" | "TryInto" | "FromIterator") {
        return None;
    }
    let value = match name {
        "Some" => Some("std::option::Option::Some"),
        "None" => Some("std::option::Option::None"),
        "Ok" => Some("std::result::Result::Ok"),
        "Err" => Some("std::result::Result::Err"),
        "drop" => Some("std::mem::drop"),
        "size_of" => Some("std::mem::size_of"),
        "size_of_val" => Some("std::mem::size_of_val"),
        "align_of" => Some("std::mem::align_of"),
        "align_of_val" => Some("std::mem::align_of_val"),
        _ => None,
    };
    let ty = match name {
        "Option" => Some("std::option::Option"),
        "Result" => Some("std::result::Result"),
        "Vec" => Some("std::vec::Vec"),
        "String" => Some("std::string::String"),
        "Box" => Some("std::boxed::Box"),
        "ToString" => Some("std::string::ToString"),
        "ToOwned" => Some("std::borrow::ToOwned"),
        "Clone" => Some("std::clone::Clone"),
        "Copy" => Some("std::marker::Copy"),
        "Send" => Some("std::marker::Send"),
        "Sync" => Some("std::marker::Sync"),
        "Sized" => Some("std::marker::Sized"),
        "Unpin" => Some("std::marker::Unpin"),
        "Default" => Some("std::default::Default"),
        "Drop" => Some("std::ops::Drop"),
        "Fn" => Some("std::ops::Fn"),
        "FnMut" => Some("std::ops::FnMut"),
        "FnOnce" => Some("std::ops::FnOnce"),
        "Eq" => Some("std::cmp::Eq"),
        "PartialEq" => Some("std::cmp::PartialEq"),
        "Ord" => Some("std::cmp::Ord"),
        "PartialOrd" => Some("std::cmp::PartialOrd"),
        "AsRef" => Some("std::convert::AsRef"),
        "AsMut" => Some("std::convert::AsMut"),
        "From" => Some("std::convert::From"),
        "Into" => Some("std::convert::Into"),
        "TryFrom" => Some("std::convert::TryFrom"),
        "TryInto" => Some("std::convert::TryInto"),
        "Iterator" => Some("std::iter::Iterator"),
        "IntoIterator" => Some("std::iter::IntoIterator"),
        "DoubleEndedIterator" => Some("std::iter::DoubleEndedIterator"),
        "ExactSizeIterator" => Some("std::iter::ExactSizeIterator"),
        "Extend" => Some("std::iter::Extend"),
        "FromIterator" => Some("std::iter::FromIterator"),
        "Future" => Some("std::future::Future"),
        "IntoFuture" => Some("std::future::IntoFuture"),
        _ => None,
    };
    match ns {
        Ns::Value => value,
        Ns::Type => ty,
        Ns::Macro => None,
    }
}

/// The primitive types (usable as path roots: `u32::from`, `str::from_utf8`).
pub(crate) fn primitive(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}
