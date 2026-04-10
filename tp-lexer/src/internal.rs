use core::{any::TypeId, fmt::Debug, num::*};

use crate::{FieldClassifier, FieldType, ToI64};

#[derive(Debug, Clone, Copy)]
pub(crate) struct IntegerToI64<T> {
    _marker: core::marker::PhantomData<T>,
}
impl<T: Debug> IntegerToI64<T> {
    pub(crate) const fn new() -> Self {
        IntegerToI64 {
            _marker: core::marker::PhantomData,
        }
    }
}

impl<T: Debug + 'static + Send + Sync> ToI64 for IntegerToI64<T> {
    fn to_i64(&self, bytes: &[u8], offset: usize) -> Result<i64, &'static str> {
        let ty_id = TypeId::of::<T>();
        let size = core::mem::size_of::<T>();
        if offset + size > bytes.len() {
            return Err("insufficient bytes to read integer");
        }
        let slice = &bytes[offset..offset + size];
        let value = if ty_id == TypeId::of::<i8>() {
            i8::from_ne_bytes([slice[0]]) as i64
        } else if ty_id == TypeId::of::<i16>() {
            i16::from_ne_bytes([slice[0], slice[1]]) as i64
        } else if ty_id == TypeId::of::<i32>() {
            i32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64
        } else if ty_id == TypeId::of::<i64>() {
            i64::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64
        } else if ty_id == TypeId::of::<i128>() {
            i128::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                slice[8], slice[9], slice[10], slice[11], slice[12], slice[13], slice[14],
                slice[15],
            ]) as i64
        } else if ty_id == TypeId::of::<isize>() {
            #[cfg(target_pointer_width = "64")]
            {
                isize::from_ne_bytes([
                    slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                ]) as i64
            }
            #[cfg(target_pointer_width = "32")]
            {
                isize::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64
            }
        } else if ty_id == TypeId::of::<u8>() {
            u8::from_ne_bytes([slice[0]]) as i64
        } else if ty_id == TypeId::of::<u16>() {
            u16::from_ne_bytes([slice[0], slice[1]]) as i64
        } else if ty_id == TypeId::of::<u32>() {
            u32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64
        } else if ty_id == TypeId::of::<u64>() {
            u64::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64
        } else if ty_id == TypeId::of::<u128>() {
            u128::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                slice[8], slice[9], slice[10], slice[11], slice[12], slice[13], slice[14],
                slice[15],
            ]) as i64
        } else if ty_id == TypeId::of::<usize>() {
            #[cfg(target_pointer_width = "64")]
            {
                usize::from_ne_bytes([
                    slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                ]) as i64
            }
            #[cfg(target_pointer_width = "32")]
            {
                usize::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64
            }
        } else if ty_id == TypeId::of::<bool>() {
            if slice[0] == 0 { 0i64 } else { 1i64 }
        } else if ty_id == TypeId::of::<char>() {
            let c = char::from_u32(u32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]))
                .ok_or("invalid char bytes")?;
            c as i64
        } else if ty_id == TypeId::of::<NonZeroI8>() {
            let v = i8::from_ne_bytes([slice[0]]) as i64;
            if v == 0 {
                return Err("NonZeroI8 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroI16>() {
            let v = i16::from_ne_bytes([slice[0], slice[1]]) as i64;
            if v == 0 {
                return Err("NonZeroI16 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroI32>() {
            let v = i32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64;
            if v == 0 {
                return Err("NonZeroI32 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroI64>() {
            let v = i64::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64;
            if v == 0 {
                return Err("NonZeroI64 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroI128>() {
            let v = i128::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                slice[8], slice[9], slice[10], slice[11], slice[12], slice[13], slice[14],
                slice[15],
            ]) as i64;
            if v == 0 {
                return Err("NonZeroI128 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroIsize>() {
            #[cfg(target_pointer_width = "64")]
            let v = isize::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64;
            #[cfg(target_pointer_width = "32")]
            let v = isize::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64;
            if v == 0 {
                return Err("NonZeroIsize cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroU8>() {
            let v = u8::from_ne_bytes([slice[0]]) as i64;
            if v == 0 {
                return Err("NonZeroU8 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroU16>() {
            let v = u16::from_ne_bytes([slice[0], slice[1]]) as i64;
            if v == 0 {
                return Err("NonZeroU16 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroU32>() {
            let v = u32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64;
            if v == 0 {
                return Err("NonZeroU32 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroU64>() {
            let v = u64::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64;
            if v == 0 {
                return Err("NonZeroU64 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroU128>() {
            let v = u128::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
                slice[8], slice[9], slice[10], slice[11], slice[12], slice[13], slice[14],
                slice[15],
            ]) as i64;
            if v == 0 {
                return Err("NonZeroU128 cannot be zero");
            }
            v
        } else if ty_id == TypeId::of::<NonZeroUsize>() {
            #[cfg(target_pointer_width = "64")]
            let v = usize::from_ne_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]) as i64;
            #[cfg(target_pointer_width = "32")]
            let v = usize::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]) as i64;
            if v == 0 {
                return Err("NonZeroUsize cannot be zero");
            }
            v
        } else {
            return Err("unsupported integer type");
        };
        Ok(value)
    }

    fn byte_len(&self) -> Option<usize> {
        Some(core::mem::size_of::<T>())
    }
}

macro_rules! integer {
    ($($t:ty),*) => {
        $(impl FieldClassifier for $t {
            const FIELD_TYPE: FieldType = FieldType::Integer(&IntegerToI64::<$t>::new());
        })*
    };
}

integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
integer!(bool, char);
integer!(
    NonZeroI8,
    NonZeroI16,
    NonZeroI32,
    NonZeroI64,
    NonZeroI128,
    NonZeroIsize
);
integer!(
    NonZeroU8,
    NonZeroU16,
    NonZeroU32,
    NonZeroU64,
    NonZeroU128,
    NonZeroUsize
);

macro_rules! unsupported {
    ($($t:ty),*) => {
        $(impl FieldClassifier for $t {
            const FIELD_TYPE: FieldType = FieldType::Unsupported;
        })*
    };
}

unsupported!(f32, f64, ());

impl<T: FieldClassifier> FieldClassifier for [T] {
    const FIELD_TYPE: FieldType = FieldType::Bytes;
}
impl<T: FieldClassifier, const SIZE: usize> FieldClassifier for [T; SIZE] {
    const FIELD_TYPE: FieldType = FieldType::Bytes;
}

impl<T: FieldClassifier> FieldClassifier for Option<T> {
    const FIELD_TYPE: FieldType = T::FIELD_TYPE;
}
impl<T: FieldClassifier, E: FieldClassifier> FieldClassifier for Result<T, E> {
    const FIELD_TYPE: FieldType = T::FIELD_TYPE;
}
impl<T: FieldClassifier> FieldClassifier for core::num::Wrapping<T> {
    const FIELD_TYPE: FieldType = T::FIELD_TYPE;
}
impl<T: FieldClassifier> FieldClassifier for core::cell::Cell<T> {
    const FIELD_TYPE: FieldType = T::FIELD_TYPE;
}

// skip RefCell

impl<T0: FieldClassifier> FieldClassifier for (T0,) {
    const FIELD_TYPE: FieldType = T0::FIELD_TYPE;
}

const fn is_unsupported<T: FieldClassifier>() -> bool {
    matches!(T::FIELD_TYPE, FieldType::Unsupported)
}

macro_rules! tuple_impls {
    ($($len:expr => ($($T:ident),+)),+) => {
        $(
            impl<$($T: FieldClassifier),+> FieldClassifier for ($($T),+) {
                const FIELD_TYPE: FieldType = if $(is_unsupported::<$T>() ||)+ false {
                    FieldType::Unsupported
                } else {
                    FieldType::Bytes
                };
            }
        )+
    };
}

tuple_impls!(
    2 => (T0, T1),
    3 => (T0, T1, T2),
    4 => (T0, T1, T2, T3),
    5 => (T0, T1, T2, T3, T4),
    6 => (T0, T1, T2, T3, T4, T5),
    7 => (T0, T1, T2, T3, T4, T5, T6),
    8 => (T0, T1, T2, T3, T4, T5, T6, T7),
    9 => (T0, T1, T2, T3, T4, T5, T6, T7, T8)
);

// skip str/string/box/vec/arc
