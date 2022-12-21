use std::mem;

/// Our int keys are simply the big-endian representation bytes for unsigned ints,
/// but "sign-flipped" (xored msb) big-endian bytes for signed ints.
///
/// So that the representation of signed integers is in the right lexicographical order.
pub trait IntKey: Sized + Copy {
    type Buf: AsRef<[u8]> + AsMut<[u8]> + Into<Vec<u8>> + Default;

    fn to_cw_bytes(&self) -> Self::Buf;
    fn from_cw_bytes(bytes: Self::Buf) -> Self;
}

macro_rules! cw_uint_keys {
    (for $($t:ty),+) => {
        $(impl IntKey for $t {
            type Buf = [u8; mem::size_of::<$t>()];

            #[inline]
            fn to_cw_bytes(&self) -> Self::Buf {
                self.to_be_bytes()
            }

            #[inline]
            fn from_cw_bytes(bytes: Self::Buf) -> Self {
                Self::from_be_bytes(bytes)
            }
        })*
    }
}

cw_uint_keys!(for u8, u16, u32, u64, u128);

macro_rules! cw_int_keys {
    (for $($t:ty, $ut:ty),+) => {
        $(impl IntKey for $t {
            type Buf = [u8; mem::size_of::<$t>()];

            #[inline]
            fn to_cw_bytes(&self) -> Self::Buf {
                (*self as $ut ^ <$t>::MIN as $ut).to_be_bytes()
            }

            #[inline]
            fn from_cw_bytes(bytes: Self::Buf) -> Self {
                (Self::from_be_bytes(bytes) as $ut ^ <$t>::MIN as $ut) as _
            }
        })*
    }
}

cw_int_keys!(for i8, u8, i16, u16, i32, u32, i64, u64, i128, u128);
