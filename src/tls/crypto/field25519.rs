//! Canonical arithmetic modulo 2^255-19, with five base-2^51 limbs.
//! All loops/indexes depend on fixed field dimensions, never operand values.
const B: u128 = 1 << 51;
const MASK: u128 = B - 1;
pub(crate) struct Field([u64; 5]);
struct Wide([u128; 5]);

impl Field {
    pub(crate) fn small(value: u32) -> Self {
        Self([u64::from(value), 0, 0, 0, 0])
    }

    /// RFC 7748 masks bit 255 and accepts all remaining noncanonical encodings.
    pub(crate) fn from_bytes(bytes: &[u8; 32]) -> Self {
        let mut limbs = Wide([0; 5]);
        for bit in 0..255 {
            limbs.0[bit / 51] |= u128::from((bytes[bit / 8] >> (bit % 8)) & 1) << (bit % 51);
        }
        normalize(limbs)
    }

    pub(crate) fn write_bytes(&self, output: &mut [u8; 32]) {
        output.fill(0);
        for bit in 0..255 {
            output[bit / 8] |= (((self.0[bit / 51] >> (bit % 51)) & 1) as u8) << (bit % 8);
        }
    }

    pub(crate) fn add(&self, other: &Self) -> Self {
        normalize(Wide(std::array::from_fn(|i| {
            u128::from(self.0[i]) + u128::from(other.0[i])
        })))
    }

    pub(crate) fn sub(&self, other: &Self) -> Self {
        normalize(Wide(std::array::from_fn(|i| {
            // Adding 2p permits unsigned subtraction for every canonical limb.
            let prime = if i == 0 { B - 19 } else { B - 1 };
            u128::from(self.0[i]) + 2 * prime - u128::from(other.0[i])
        })))
    }

    #[inline(never)]
    pub(crate) fn mul(&self, other: &Self) -> Self {
        let mut product = Wide([0; 5]);
        for left in 0..5 {
            for right in 0..5 {
                let degree = left + right;
                let factor = if degree < 5 { 1 } else { 19 };
                product.0[degree % 5] +=
                    u128::from(self.0[left]) * u128::from(other.0[right]) * factor;
            }
        }
        normalize(product)
    }

    pub(crate) fn square(&self) -> Self {
        self.mul(self)
    }

    pub(crate) fn inverse(&self) -> Self {
        // Fixed exponent p-2 = 2^255-21. Zero deliberately produces zero.
        let mut value = Self::small(1);
        for bit in (0..255).rev() {
            value = value.square();
            if bit >= 5 || (0xeb_u8 >> bit) & 1 != 0 {
                value = value.mul(self);
            }
        }
        value
    }

    #[inline(never)]
    pub(crate) fn swap(left: &mut Self, right: &mut Self, bit: u8) {
        let mask = 0_u64.wrapping_sub(u64::from(bit & 1));
        for index in 0..5 {
            let change = (left.0[index] ^ right.0[index]) & mask;
            left.0[index] ^= change;
            right.0[index] ^= change;
        }
    }
}

fn normalize(mut limbs: Wide) -> Field {
    // A product coefficient is below 77B^2 < 2^109. The first pass leaves
    // limb zero below 1483B; the second below B+19; the third resolves wrap.
    for _ in 0..3 {
        for index in 0..4 {
            limbs.0[index + 1] += limbs.0[index] >> 51;
            limbs.0[index] &= MASK;
        }
        let carry = limbs.0[4] >> 51;
        limbs.0[4] &= MASK;
        limbs.0[0] += carry * 19;
    }
    // The normalized value is below 2^255. Carry from h+19 selects h-p.
    let mut candidate = Wide([0; 5]);
    let mut carry = 19;
    for index in 0..5 {
        let sum = limbs.0[index] + carry;
        candidate.0[index] = sum & MASK;
        carry = sum >> 51;
    }
    let select = 0_u128.wrapping_sub(carry);
    Field(std::array::from_fn(|i| {
        ((limbs.0[i] & !select) | (candidate.0[i] & select)) as u64
    }))
}

impl Drop for Field {
    fn drop(&mut self) {
        super::secret::erase(&mut self.0);
    }
}
impl Drop for Wide {
    fn drop(&mut self) {
        super::secret::erase(&mut self.0);
    }
}
