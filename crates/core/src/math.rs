use crate::{Error, Result};
// The upstream macro implements integer internals with explicit division.
#[allow(clippy::manual_div_ceil)]
mod wide {
    uint::construct_uint! { pub struct U256(4); }
}
pub use wide::U256;

fn narrow(value: U256) -> Result<u128> {
    if value > U256::from(u128::MAX) {
        return Err(Error::Arithmetic);
    }
    Ok(value.low_u128())
}

pub fn mul_div_floor(a: u128, b: u128, divisor: u128) -> Result<u128> {
    if divisor == 0 {
        return Err(Error::Arithmetic);
    }
    narrow(U256::from(a) * U256::from(b) / U256::from(divisor))
}

pub fn mul_div_ceil(a: u128, b: u128, divisor: u128) -> Result<u128> {
    if divisor == 0 {
        return Err(Error::Arithmetic);
    }
    let product = U256::from(a) * U256::from(b);
    let divisor = U256::from(divisor);
    let quotient = product / divisor;
    narrow(quotient + U256::from(u8::from(!(product % divisor).is_zero())))
}

pub fn sqrt_product(a: u128, b: u128) -> u128 {
    let n = U256::from(a) * U256::from(b);
    if n.is_zero() {
        return 0;
    }
    let mut x = U256::one() << n.bits().div_ceil(2);
    loop {
        let next = (x + n / x) >> 1;
        if next >= x {
            return x.low_u128();
        }
        x = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_math_handles_boundaries_and_rounding() {
        assert_eq!(
            mul_div_floor(u128::MAX, u128::MAX, u128::MAX),
            Ok(u128::MAX)
        );
        assert_eq!(mul_div_floor(5, 3, 2), Ok(7));
        assert_eq!(mul_div_ceil(5, 3, 2), Ok(8));
        assert_eq!(mul_div_ceil(4, 3, 2), Ok(6));
        assert_eq!(mul_div_ceil(0, 3, 2), Ok(0));
        assert_eq!(mul_div_floor(1, 1, 0), Err(Error::Arithmetic));
        assert_eq!(mul_div_ceil(u128::MAX, 2, 1), Err(Error::Arithmetic));
        assert_eq!(sqrt_product(u128::MAX, u128::MAX), u128::MAX);
        assert_eq!(sqrt_product(2, 3), 2);
        assert_eq!(sqrt_product(0, 3), 0);
    }
}
