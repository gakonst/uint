#![allow(clippy::module_name_repetitions)]

/// Knuth division
use core::{convert::TryFrom, u64};

/// Compute a + b + carry, returning the result and the new carry over.
const fn adc(a: u64, b: u64, carry: u64) -> (u64, u64) {
    let ret = (a as u128) + (b as u128) + (carry as u128);
    // We want truncation here
    #[allow(clippy::cast_possible_truncation)]
    (ret as u64, (ret >> 64) as u64)
}

/// Compute a - (b * c + borrow), returning the result and the new borrow.
const fn msb(a: u64, b: u64, c: u64, borrow: u64) -> (u64, u64) {
    let ret = (a as u128).wrapping_sub((b as u128) * (c as u128) + (borrow as u128));
    // TODO: Why is this wrapping_sub required?
    // We want truncation here
    #[allow(clippy::cast_possible_truncation)]
    (ret as u64, 0_u64.wrapping_sub((ret >> 64) as u64))
}

const fn val_2(lo: u64, hi: u64) -> u128 {
    ((hi as u128) << 64) | (lo as u128)
}

const fn mul_2(a: u64, b: u64) -> u128 {
    (a as u128) * (b as u128)
}

/// Compute <hi, lo> / d, returning the quotient and the remainder.
// TODO: Make sure it uses divq on x86_64.
// See http://lists.llvm.org/pipermail/llvm-dev/2017-October/118323.html
// (Note that we require d > hi for this)
// TODO: If divq is not supported, use a fast software implementation:
// See https://gmplib.org/~tege/division-paper.pdf
fn divrem_2by1(lo: u64, hi: u64, d: u64) -> (u64, u64) {
    debug_assert!(d > 0);
    debug_assert!(d > hi);
    let d = u128::from(d);
    let n = val_2(lo, hi);
    let q = n / d;
    let r = n % d;
    debug_assert!(q < val_2(0, 1));
    debug_assert!(
        mul_2(u64::try_from(q).unwrap(), u64::try_from(d).unwrap())
            + val_2(u64::try_from(r).unwrap(), 0)
            == val_2(lo, hi)
    );
    debug_assert!(r < d);
    // There should not be any truncation.
    #[allow(clippy::cast_possible_truncation)]
    (q as u64, r as u64)
}

#[allow(clippy::cast_possible_truncation)] // Intentional
pub fn divrem_nby1(numerator: &mut [u64], divisor: u64) -> u64 {
    debug_assert!(divisor > 0);
    let mut remainder = 0;
    for limb in numerator.iter_mut().rev() {
        remainder <<= 64;
        remainder |= u128::from(*limb);
        *limb = (remainder / u128::from(divisor)) as u64;
        remainder %= u128::from(divisor);
    }
    remainder as u64
}

//      |  n2 n1 n0  |
//  q = |  --------  |
//      |_    d1 d0 _|
fn div_3by2(n: &[u64; 3], d: &[u64; 2]) -> u64 {
    // The highest bit of d needs to be set
    debug_assert!(d[1] >> 63 == 1);

    // The quotient needs to fit u64. For this we need [n2 n1] < [d1 d0]
    debug_assert!(val_2(n[1], n[2]) < val_2(d[0], d[1]));

    if n[2] == d[1] {
        // From [n2 n1] < [d1 d0] and n2 = d1 it follows that n[1] < d[0].
        debug_assert!(n[1] < d[0]);
        // We start by subtracting 2^64 times the divisor, resulting in a
        // negative remainder. Depending on the result, we need to add back
        // in one or two times the divisor to make the remainder positive.
        // (It can not be more since the divisor is > 2^127 and the negated
        // remainder is < 2^128.)
        let neg_remainder = val_2(0, d[0]) - val_2(n[0], n[1]);
        if neg_remainder > val_2(d[0], d[1]) {
            0xffff_ffff_ffff_fffe_u64
        } else {
            0xffff_ffff_ffff_ffff_u64
        }
    } else {
        // Compute quotient and remainder
        let (mut q, mut r) = divrem_2by1(n[1], n[2], d[1]);

        if mul_2(q, d[0]) > val_2(n[0], r) {
            q -= 1;
            r = r.wrapping_add(d[1]);
            let overflow = r < d[1];
            if !overflow && mul_2(q, d[0]) > val_2(n[0], r) {
                q -= 1;
                // UNUSED: r += d[1];
            }
        }
        q
    }
}

/// ⚠️ Division with remainder.
///
/// **Warning.** This function is not part of the stable API.
///
/// The quotient is stored in the `numerator` and the remainder is stored
/// in the `divisor`.
///
/// # Algorithms
///
/// It uses schoolbook division when the `divisor` first a single limb,
/// otherwise it uses Knuth's algorithm D.
///
/// # Panics
///
/// Panics if `divisor` is zero.
pub fn div_rem(numerator: &mut [u64], divisor: &mut [u64]) {
    assert!(!divisor.is_empty());

    // Trim most significant zeros from divisor.
    let i = divisor
        .iter()
        .rposition(|&x| x != 0)
        .expect("Divisor is zero");
    let divisor = &mut divisor[..=i];

    // Compute result
    if divisor.len() == 1 {
        let remainder = divrem_nby1(numerator, divisor[0]);

        // Copy remainder to divisor (always fits)
        divisor[0] = remainder;
        for limb in &mut divisor[1..] {
            *limb = 0;
        }
    } else {
        // Zero extend numerator
        let mut buffer = Vec::with_capacity(numerator.len() + 1);
        buffer.extend_from_slice(numerator);
        buffer.push(0);

        divrem_nbym(&mut buffer, divisor);
        let buf_div = &buffer[..divisor.len()];
        let buf_rem = &buffer[divisor.len()..];

        // Copy remainder to divisor
        debug_assert_eq!(buf_div.len(), divisor.len());
        divisor.copy_from_slice(buf_div);

        // Copy quotient to numerator
        if buf_rem.len() > numerator.len() {
            numerator.copy_from_slice(&buf_rem[..numerator.len()]);
            for limb in &buf_rem[numerator.len()..] {
                debug_assert_eq!(*limb, 0);
            }
        } else {
            numerator[..buf_rem.len()].copy_from_slice(buf_rem);
            for limb in &mut numerator[buf_rem.len()..] {
                *limb = 0;
            }
        }
    }
}

/// Turns numerator into remainder, returns quotient.
///
/// Implements Knuth's division algorithm.
/// See D. Knuth "The Art of Computer Programming". Sec. 4.3.1. Algorithm D.
/// See <https://github.com/chfast/intx/blob/master/lib/intx/div.cpp>
///
/// `divisor` must have non-zero first limbs. Consequently, the remainder is
/// length at most `divisor.len()`, and the qouient is at most
/// `numerator.len() - divisor.len()` limbs.
///
/// NOTE: numerator must have one additional zero at the end.
/// The result will be computed in-place in numerator.
/// The divisor will be normalized.
pub fn divrem_nbym(numerator: &mut [u64], divisor: &mut [u64]) {
    debug_assert!(divisor.len() >= 2);
    debug_assert!(numerator.len() > divisor.len());
    debug_assert!(*divisor.last().unwrap() > 0);
    debug_assert!(*numerator.last().unwrap() == 0);
    // OPT: Once const generics are in, unroll for lengths.
    // OPT: We can use macro generated specializations till then.
    let n = divisor.len();
    let m = numerator.len() - n - 1;

    // D1. Normalize.
    let shift = divisor[n - 1].leading_zeros();
    if shift > 0 {
        numerator[n + m] = numerator[n + m - 1] >> (64 - shift);
        for i in (1..n + m).rev() {
            numerator[i] <<= shift;
            numerator[i] |= numerator[i - 1] >> (64 - shift);
        }
        numerator[0] <<= shift;
        for i in (1..n).rev() {
            divisor[i] <<= shift;
            divisor[i] |= divisor[i - 1] >> (64 - shift);
        }
        divisor[0] <<= shift;
    }

    // D2. Loop over quotient digits
    for j in (0..=m).rev() {
        // D3. Calculate approximate quotient word
        let mut qhat = div_3by2(
            &[numerator[j + n - 2], numerator[j + n - 1], numerator[j + n]],
            &[divisor[n - 2], divisor[n - 1]],
        );

        // D4. Multiply and subtract.
        let mut borrow = 0;
        for i in 0..n {
            let (a, b) = msb(numerator[j + i], qhat, divisor[i], borrow);
            numerator[j + i] = a;
            borrow = b;
        }

        // D5. Test remainder for negative result.
        if numerator[j + n] < borrow {
            // D6. Add back. (happens rarely)
            let mut carry = 0;
            for i in 0..n {
                let (a, b) = adc(numerator[j + i], divisor[i], carry);
                numerator[j + i] = a;
                carry = b;
            }
            qhat -= 1;
            // The updated value of numerator[j + n] would be 0. But since we're going to
            // overwrite it below, we only check that the result would be 0.
            debug_assert_eq!(numerator[j + n].wrapping_sub(borrow).wrapping_add(carry), 0);
        } else {
            // This the would be the updated value when the remainder is non-negative.
            debug_assert_eq!(numerator[j + n].wrapping_sub(borrow), 0);
        }

        // Store remainder in the unused bits of numerator
        numerator[j + n] = qhat;
    }

    // D8. Unnormalize.
    if shift > 0 {
        // Make sure to only normalize the remainder part, the quotient
        // is already normalized.
        for i in 0..(n - 1) {
            numerator[i] >>= shift;
            numerator[i] |= numerator[i + 1] << (64 - shift);
        }
        numerator[n - 1] >>= shift;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: u64 = 1_u64 << 63;
    const FULL: u64 = u64::max_value();

    #[test]
    fn div_3by2_tests() {
        // Test cases where n[2] == d[1]
        assert_eq!(div_3by2(&[FULL, FULL - 1, HALF], &[FULL, HALF]), FULL);
        assert_eq!(div_3by2(&[0, 0, HALF], &[FULL, HALF]), FULL - 1);
    }

    #[test]
    fn test_divrem_4by3() {
        let mut numerator = [40, 31, 79, 84, 0];
        let mut divisor = [53, 12, 12];
        let expected_quotient = [u64::max_value(), 6];
        let expected_remainder = [93, 0xffff_ffff_ffff_feb8, 6];
        divrem_nbym(&mut numerator, &mut divisor);
        let remainder = &numerator[0..3];
        let quotient = &numerator[3..5];
        assert_eq!(remainder, expected_remainder);
        assert_eq!(quotient, expected_quotient);
    }

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn test_divrem_8by4() {
        let mut numerator = [
            0x9c2bcebfa9cca2c6_u64,
            0x274e154bb5e24f7a_u64,
            0xe1442d5d3842be2b_u64,
            0xf18f5adfd420853f_u64,
            0x04ed6127eba3b594_u64,
            0xc5c179973cdb1663_u64,
            0x7d7f67780bb268ff_u64,
            0x0000000000000003_u64,
            0x0000000000000000_u64,
        ];
        let mut divisor = [
            0x0181880b078ab6a1_u64,
            0x62d67f6b7b0bda6b_u64,
            0x92b1840f9c792ded_u64,
            0x0000000000000019_u64,
        ];
        let expected_quotient = [
            0x9128464e61d6b5b3_u64,
            0xd9eea4fc30c5ac6c_u64,
            0x944a2d832d5a6a08_u64,
            0x22f06722e8d883b1_u64,
            0x0000000000000000_u64,
        ];
        let expected_remainder = [
            0x1dfa5a7ea5191b33_u64,
            0xb5aeb3f9ad5e294e_u64,
            0xfc710038c13e4eed_u64,
            0x000000000000000b_u64,
        ];
        divrem_nbym(&mut numerator, &mut divisor);
        let remainder = &numerator[0..4];
        let quotient = &numerator[4..9];
        assert_eq!(remainder, expected_remainder);
        assert_eq!(quotient, expected_quotient);
    }

    #[test]
    #[allow(clippy::unreadable_literal)]
    fn test_divrem_4by4() {
        let mut numerator = [
            0xe72530a3d4e91ea3,
            0x4edef514135f5899,
            0x1868b9a7d418e9c6,
            0x6f1480e63854afa4,
            0,
        ];
        let mut divisor = [
            0xa62b65900d2a62bb,
            0xffb08af4108f9aea,
            0xb87126f34ee28533,
            0x3ba5ddaec5090ef0,
        ];
        divrem_nbym(&mut numerator, &mut divisor);
        let remainder = &numerator[0..4];
        let quotient = numerator[4];
        assert_eq!(remainder, [
            0x40f9cb13c7bebbe8,
            0x4f2e6a2002cfbdaf,
            0x5ff792b485366492,
            0x336ea337734ba0b3
        ]);
        assert_eq!(quotient, 1);
    }

    // proptest!(
    // #[test]
    // fn div_3by2_correct(q: u64, d0: u64, d1: u64) {
    // TODO: Add remainder
    // let d1 = d1 | (1 << 63);
    // let n = U256::from_limbs([d0, d1, 0, 0]) * &U256::from(q);
    // debug_assert!(n.limb(3) == 0);
    // let qhat = div_3by2(&[n.limb(0), n.limb(1), n.limb(2)], &[d0, d1]);
    // prop_assert_eq!(qhat, q);
    // }
    // );
}
