#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(unused_imports)]

use core::convert::TryFrom;

/// BLAKE2s context (unkeyed).
#[repr(align(32))]
pub struct Blake2s {
    h: [u32; 8],
    buf: [u8; BUF_LEN],
    ctr: u64,
    out_len: usize,
}

/// BLAKE2s context (with a key). The key is saved internally, so that
/// multiple successive hashing operations can be performed with the same
/// context without reinjecting the key each time.
#[repr(align(32))]
pub struct KeyedBlake2s {
    ctx: Blake2s,
    saved_key: [u8; 32],
    saved_key_len: usize,
}

const BUF_LEN: usize = 64;

/// Convenience wrapper for BLAKE2s (unkeyed) with a 256-bit output, which
/// is the most common combination. That wrapper offers finalization functions
/// that return the computed output as a fixed-size 32-byte array.
pub struct Blake2s256(Blake2s);

impl Blake2s256 {

    /// Initialize a new context.
    #[inline(always)]
    pub fn new() -> Self {
        Self(Blake2s::new(32))
    }

    /// Inject some more bytes into the context.
    #[inline(always)]
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    /// Finalize the current computation and get a 32-byte output.
    /// The context MUST NOT be used afterwards without first resetting it.
    #[inline(always)]
    pub fn finalize(&mut self) -> [u8; 32] {
        self.0.inner_finalize()
    }

    /// Finalize the current computation and get a 32-byte output.
    /// The context is automatically reset, so that it can be used again
    /// for a new computation.
    #[inline(always)]
    pub fn finalize_reset(&mut self) -> [u8; 32] {
        self.0.inner_finalize_reset()
    }

    /// Finalize this context and get the output. The output (32 bytes)
    /// is written into the provided slice. The output size (32) is returned.
    /// The context is NOT reset and must not be used for further hashing.
    #[inline(always)]
    pub fn finalize_write(&mut self, out: &mut [u8]) -> usize {
        self.0.finalize_write(out)
    }

    /// Finalize this context and get the output. The output (32 bytes)
    /// is written into the provided slice. The output size (32) is returned.
    /// The context is automatically reset and can be used for a new
    /// hashing operation.
    #[inline(always)]
    pub fn finalize_reset_write(&mut self, out: &mut [u8]) -> usize {
        self.0.finalize_reset_write(out)
    }

    /// One-stop function for hashing some input into a 32-byte output.
    #[inline(always)]
    pub fn hash(data: &[u8]) -> [u8; 32] {
        let mut sh = Self::new();
        sh.update(data);
        sh.finalize()
    }
}

impl KeyedBlake2s {

    /// Initialize the context. The output length (in bytes) must be
    /// between 1 and 32. The key length must be between 0 and 32 bytes;
    /// if the key has length 0, then this is equivalent to unkeyed
    /// hashing.
    pub fn new(out_len: usize, key: &[u8]) -> Self {
        assert!(key.len() <= 32);
        let mut ctx = Blake2s::new(out_len);
        let mut saved_key = [0u8; 32];
        let saved_key_len = key.len();
        if saved_key_len > 0 {
            ctx.h[0] ^= (saved_key_len as u32) << 8;
            saved_key[..saved_key_len].copy_from_slice(key);
            ctx.buf[..saved_key_len].copy_from_slice(key);
            ctx.ctr = BUF_LEN as u64;
        }
        Self { ctx, saved_key, saved_key_len }
    }

    /// Inject some more bytes into the context.
    #[inline(always)]
    pub fn update(&mut self, data: &[u8]) {
        self.ctx.update(data);
    }

    /// Reset this context.
    #[inline]
    pub fn reset(&mut self) {
        self.ctx.reset();
        if self.saved_key_len > 0 {
            self.ctx.h[0] ^= (self.saved_key_len as u32) << 8;
            self.ctx.buf[..self.saved_key_len].copy_from_slice(&self.saved_key);
            self.ctx.ctr = BUF_LEN as u64;
        }
    }

    /// Finalize this context and get the output. The output (`out_len` bytes)
    /// is written into the provided slice. The output size is returned.
    /// The context is NOT reset and must not be used for further hashing.
    #[inline(always)]
    pub fn finalize_write(&mut self, out: &mut [u8]) -> usize {
        self.ctx.finalize_write(out)
    }

    /// Finalize this context and get the output. The output (`out_len` bytes)
    /// is written into the provided slice. The output size is returned.
    /// The context is automatically reset and can be used for a new
    /// hashing operation.
    #[inline(always)]
    pub fn finalize_reset_write(&mut self, out: &mut [u8]) -> usize {
        let r = self.ctx.finalize_write(out);
        self.reset();
        r
    }

    /// One-stop function for hashing some input (and a key) into an output
    /// buffer. The output length is provided explicitly; the output buffer
    /// (`out`) may be larger.
    #[inline(always)]
    pub fn hash_into(out_len: usize, key: &[u8], data: &[u8], out: &mut [u8]) {
        let mut sh = Self::new(out_len, key);
        sh.update(data);
        sh.finalize_write(out);
    }
}

impl Blake2s {

    const IV: [u32; 8] = [
        0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
        0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
    ];

    /// Initialize the context. The output length (in bytes) MUST be
    /// between 1 and 32 bytes (inclusive).
    pub fn new(out_len: usize) -> Self {
        assert!(1 <= out_len && out_len <= 32);
        let mut h = Self::IV;
        h[0] ^= 0x01010000 ^ (out_len as u32);
        Self {
            h: h,
            buf: [0u8; BUF_LEN],
            ctr: 0,
            out_len: out_len,
        }
    }

    /// Inject some more bytes into the context.
    pub fn update(&mut self, data: &[u8]) {
        // ctr == !0u64 is the marker of an invalid context.
        assert!(self.ctr != !0u64);

        if data.len() == 0 {
            return;
        }
        let mut j = 0;

        // Complete the current block, if not already full.
        let p = (self.ctr as usize) & (BUF_LEN - 1);
        if self.ctr == 0 || p != 0 {
            let clen = BUF_LEN - p;
            if clen >= data.len() {
                self.buf[p..(p + data.len())].copy_from_slice(data);
                self.ctr += data.len() as u64;
                return;
            }
            self.buf[p..].copy_from_slice(&data[..clen]);
            self.ctr += clen as u64;
            j = clen;
        }

        // Process the buffered block.
        Self::process_block(&mut self.h, &self.buf, self.ctr, false);

        // Process all subsequent full blocks, except the last.
        while j < data.len() {
            let clen = data.len() - j;
            if clen <= BUF_LEN {
                self.buf[..clen].copy_from_slice(&data[j..]);
                self.ctr += clen as u64;
                return;
            }
            self.ctr += BUF_LEN as u64;
            let j2 = j + BUF_LEN;
            Self::process_block(&mut self.h, &data[j..j2], self.ctr, false);
            j = j2;
        }
    }

    /// Reset this context.
    #[inline]
    pub fn reset(&mut self) {
        self.h[..].copy_from_slice(&Self::IV);
        self.h[0] ^= 0x01010000 ^ (self.out_len as u32);
        self.buf[..].copy_from_slice(&[0u8; BUF_LEN]);
        self.ctr = 0;
    }

    /// Finalize this context and get the output. The output (`out_len` bytes)
    /// is written into the provided slice. The output size is returned.
    /// The context is NOT reset and must not be used for further hashing.
    #[inline]
    pub fn finalize_write(&mut self, out: &mut [u8]) -> usize {
        out[..self.out_len].copy_from_slice(
            &self.inner_finalize()[..self.out_len]);
        self.out_len
    }

    /// Finalize this context and get the output. The output (`out_len` bytes)
    /// is written into the provided slice. The output size is returned.
    /// The context is automatically reset and can be used for a new
    /// hashing operation.
    #[inline]
    pub fn finalize_reset_write(&mut self, out: &mut [u8]) -> usize {
        out[..self.out_len].copy_from_slice(
            &self.inner_finalize_reset()[..self.out_len]);
        self.out_len
    }

    /// One-stop function for hashing some input into an output buffer.
    /// The output length is provided explicitly; the output buffer (`out`)
    /// may be larger.
    #[inline(always)]
    pub fn hash_into(out_len: usize, data: &[u8], out: &mut [u8]) {
        let mut sh = Self::new(out_len);
        sh.update(data);
        sh.finalize_write(out);
    }

    // Finalize this context and get a 32-byte output. Nominally, that
    // output should be truncated to the configured output size.
    fn inner_finalize(&mut self) -> [u8; 32] {
        // ctr == !0u64 is the marker of an invalid context.
        assert!(self.ctr != !0u64);

        // Pad the current block with zeros, if not full.
        let p = (self.ctr as usize) & (BUF_LEN - 1);
        if self.ctr == 0 || p != 0 {
            let zb = [0u8; BUF_LEN];
            self.buf[p..].copy_from_slice(&zb[p..]);
        }

        // Process the last (padded) block.
        Self::process_block(&mut self.h, &self.buf, self.ctr, true);

        // Write out the result.
        let mut r = [0u8; 32];
        for i in 0..8 {
            r[(4 * i)..(4 * i + 4)].copy_from_slice(&self.h[i].to_le_bytes());
        }

        // Tag the context as unusable until next reset.
        self.ctr = !0u64;
        r
    }

    // `inner_finalize()` followed by `reset()`.
    #[inline(always)]
    fn inner_finalize_reset(&mut self) -> [u8; 32] {
        let r = self.inner_finalize();
        self.reset();
        r
    }

    // Internal block processing function. 8-word state is `h`; the block
    // data is 64 bytes. The current input counter (`ctr`) is provided.
    // For the final block, `last` is `true`.
    fn process_block(h: &mut [u32; 8], block: &[u8], ctr: u64, last: bool) {
        #[cfg(not(any(
            target_arch = "x86_64")))]
        {
            let mut v = [0u32; 16];
            v[..8].copy_from_slice(&h[..]);
            v[8..].copy_from_slice(&Self::IV);
            v[12] ^= ctr as u32;
            v[13] ^= (ctr >> 32) as u32;
            if last {
                v[14] = !v[14];
            }

            let mut m = [0u32; 16];
            for i in 0..16 {
                m[i] = u32::from_le_bytes(*<&[u8; 4]>::try_from(
                    &block[(4 * i)..(4 * i + 4)]).unwrap());
            }

            macro_rules! gg {
                ($a: expr, $b: expr, $c: expr, $d: expr, $x: expr, $y: expr)
                => {
                    v[$a] = v[$a].wrapping_add(v[$b].wrapping_add($x));
                    v[$d] = (v[$d] ^ v[$a]).rotate_right(16);
                    v[$c] = v[$c].wrapping_add(v[$d]);
                    v[$b] = (v[$b] ^ v[$c]).rotate_right(12);
                    v[$a] = v[$a].wrapping_add(v[$b].wrapping_add($y));
                    v[$d] = (v[$d] ^ v[$a]).rotate_right(8);
                    v[$c] = v[$c].wrapping_add(v[$d]);
                    v[$b] = (v[$b] ^ v[$c]).rotate_right(7);
                }
            }

            macro_rules! rr {
                ($s0: expr, $s1: expr, $s2: expr, $s3: expr,
                 $s4: expr, $s5: expr, $s6: expr, $s7: expr,
                 $s8: expr, $s9: expr, $sA: expr, $sB: expr,
                 $sC: expr, $sD: expr, $sE: expr, $sF: expr)
                => {
                    gg!(0, 4,  8, 12, m[$s0], m[$s1]);
                    gg!(1, 5,  9, 13, m[$s2], m[$s3]);
                    gg!(2, 6, 10, 14, m[$s4], m[$s5]);
                    gg!(3, 7, 11, 15, m[$s6], m[$s7]);
                    gg!(0, 5, 10, 15, m[$s8], m[$s9]);
                    gg!(1, 6, 11, 12, m[$sA], m[$sB]);
                    gg!(2, 7,  8, 13, m[$sC], m[$sD]);
                    gg!(3, 4,  9, 14, m[$sE], m[$sF]);
                }
            }
            rr!( 0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15);
            rr!(14, 10,  4,  8,  9, 15, 13,  6,  1, 12,  0,  2, 11,  7,  5,  3);
            rr!(11,  8, 12,  0,  5,  2, 15, 13, 10, 14,  3,  6,  7,  1,  9,  4);
            rr!( 7,  9,  3,  1, 13, 12, 11, 14,  2,  6,  5, 10,  4,  0, 15,  8);
            rr!( 9,  0,  5,  7,  2,  4, 10, 15, 14,  1, 11, 12,  6,  8,  3, 13);
            rr!( 2, 12,  6, 10,  0, 11,  8,  3,  4, 13,  7,  5, 15, 14,  1,  9);
            rr!(12,  5,  1, 15, 14, 13,  4, 10,  0,  7,  6,  3,  9,  2,  8, 11);
            rr!(13, 11,  7, 14, 12,  1,  3,  9,  5,  0, 15,  4,  8,  6,  2, 10);
            rr!( 6, 15, 14,  9, 11,  3,  0,  8, 12,  2, 13,  7,  1,  4, 10,  5);
            rr!(10,  2,  8,  4,  7,  6,  1,  5, 15, 11,  9, 14,  3, 12, 13,  0);

            for i in 0..8 {
                h[i] ^= v[i] ^ v[i + 8];
            }
        }

        #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
        unsafe {
            // x86_64 + AVX2
            use core::arch::x86_64::*;
            use core::mem::transmute;

            let xror8 = _mm_setr_epi8(
                1, 2, 3, 0, 5, 6, 7, 4,
                9, 10, 11, 8, 13, 14, 15, 12);
            let xror16 = _mm_setr_epi8(
                2, 3, 0, 1, 6, 7, 4, 5,
                10, 11, 8, 9, 14, 15, 12, 13);

            // Initialize state.
            let xh0 = _mm_loadu_si128(transmute(&h[0]));
            let xh1 = _mm_loadu_si128(transmute(&h[4]));
            let mut xv0 = xh0;
            let mut xv1 = xh1;
            let mut xv2 = _mm_loadu_si128(transmute(&Self::IV[0]));
            let mut xv3 = _mm_loadu_si128(transmute(&Self::IV[4]));
            xv3 = _mm_xor_si128(xv3, _mm_setr_epi32(
                ctr as i32, (ctr >> 32) as i32, -(last as i32), 0));

            // Load data and move it into the proper order for the first round:
            //   xm0:  0  2  4  6
            //   xm1:  1  3  5  7
            //   xm2:  8 10 12 14
            //   xm3:  9 11 13 15
            let xm0 = _mm_loadu_si128(transmute(&block[ 0]));
            let xm1 = _mm_loadu_si128(transmute(&block[16]));
            let xm2 = _mm_loadu_si128(transmute(&block[32]));
            let xm3 = _mm_loadu_si128(transmute(&block[48]));

            let xn0 = _mm_shuffle_epi32(xm0, 0xD8);
            let xn1 = _mm_shuffle_epi32(xm1, 0xD8);
            let xm0 = _mm_unpacklo_epi64(xn0, xn1);
            let xm1 = _mm_unpackhi_epi64(xn0, xn1);

            let xn2 = _mm_shuffle_epi32(xm2, 0xD8);
            let xn3 = _mm_shuffle_epi32(xm3, 0xD8);
            let xm2 = _mm_unpacklo_epi64(xn2, xn3);
            let xm3 = _mm_unpackhi_epi64(xn2, xn3);

            macro_rules! g4 { ($xx: expr, $xy: expr) => {
                xv0 = _mm_add_epi32(xv0, _mm_add_epi32(xv1, $xx));
                xv3 = _mm_shuffle_epi8(_mm_xor_si128(xv0, xv3), xror16);
                xv2 = _mm_add_epi32(xv2, xv3);
                let xtg = _mm_xor_si128(xv1, xv2);
                xv1 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 12), _mm_slli_epi32(xtg, 20));
                xv0 = _mm_add_epi32(xv0, _mm_add_epi32(xv1, $xy));
                xv3 = _mm_shuffle_epi8(_mm_xor_si128(xv0, xv3), xror8);
                xv2 = _mm_add_epi32(xv2, xv3);
                let xtg = _mm_xor_si128(xv1, xv2);
                xv1 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 7), _mm_slli_epi32(xtg, 25));
            } }

            macro_rules! rr { ($i0: expr, $i1: expr, $i2: expr, $i3: expr) => {
                g4!($i0, $i1);
                xv1 = _mm_shuffle_epi32(xv1, 0x39);
                xv2 = _mm_shuffle_epi32(xv2, 0x4E);
                xv3 = _mm_shuffle_epi32(xv3, 0x93);
                g4!($i2, $i3);
                xv1 = _mm_shuffle_epi32(xv1, 0x93);
                xv2 = _mm_shuffle_epi32(xv2, 0x4E);
                xv3 = _mm_shuffle_epi32(xv3, 0x39);
            } }

            // round 0
            rr!(xm0, xm1, xm2, xm3);

            // round 1
            let xt0 = _mm_shuffle_epi32(xm0, 0x00);
            let xt1 = _mm_shuffle_epi32(xm0, 0xC8);
            let xt2 = _mm_shuffle_epi32(xm1, 0x70);
            let xt3 = _mm_shuffle_epi32(xm1, 0x80);
            let xt4 = _mm_shuffle_epi32(xm2, 0x01);
            let xt5 = _mm_shuffle_epi32(xm2, 0x02);
            let xt6 = _mm_shuffle_epi32(xm2, 0x03);
            let xt7 = _mm_shuffle_epi32(xm3, 0x80);
            let xt8 = _mm_shuffle_epi32(xm3, 0x10);
            let xt9 = _mm_shuffle_epi32(xm3, 0x30);
            let xn0 = _mm_blend_epi32(
                _mm_blend_epi32(xt6, xt1, 0x02),
                xt7, 0x0C);
            let xn1 = _mm_blend_epi32(
                _mm_blend_epi32(xt4, xt9, 0x04),
                xt1, 0x08);
            let xn2 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt0, 0x02),
                xt8, 0x04);
            let xn3 = _mm_blend_epi32(
                _mm_blend_epi32(xt5, xm0, 0x02),
                xt2, 0x0C);
            rr!(xn0, xn1, xn2, xn3);

            // round 2
            let xt0 = _mm_shuffle_epi32(xn0, 0x40);
            let xt1 = _mm_shuffle_epi32(xn0, 0x80);
            let xt2 = _mm_shuffle_epi32(xn1, 0x80);
            let xt3 = _mm_shuffle_epi32(xn1, 0x0D);
            let xt4 = _mm_shuffle_epi32(xn2, 0x04);
            let xt5 = _mm_shuffle_epi32(xn2, 0x32);
            let xt6 = _mm_shuffle_epi32(xn3, 0x10);
            let xt7 = _mm_shuffle_epi32(xn3, 0x2C);
            let xm0 = _mm_blend_epi32(
                _mm_blend_epi32(xt5, xt6, 0x02),
                xt2, 0x08);
            let xm1 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt4, 0x02),
                _mm_blend_epi32(xt6, xn0, 0x08), 0x0C);
            let xm2 = _mm_blend_epi32(
                _mm_blend_epi32(xt2, xt7, 0x06),
                xt1, 0x08);
            let xm3 = _mm_blend_epi32(
                _mm_blend_epi32(xt0, xt3, 0x02),
                xt4, 0x04);
            rr!(xm0, xm1, xm2, xm3);

            // round 3
            let xt0 = _mm_shuffle_epi32(xm0, 0x10);
            let xt1 = _mm_shuffle_epi32(xm0, 0xC8);
            let xt2 = _mm_shuffle_epi32(xm1, 0x10);
            let xt3 = _mm_shuffle_epi32(xm1, 0x32);
            let xt4 = _mm_shuffle_epi32(xm2, 0x03);
            let xt5 = _mm_shuffle_epi32(xm2, 0x06);
            let xt6 = _mm_shuffle_epi32(xm3, 0x39);
            let xn0 = _mm_blend_epi32(
                _mm_blend_epi32(xt5, xt3, 0x04),
                xt0, 0x08);
            let xn1 = _mm_blend_epi32(
                _mm_blend_epi32(xt4, xt6, 0x0A),
                xt0, 0x04);
            let xn2 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt1, 0x0A),
                xt6, 0x04);
            let xn3 = _mm_blend_epi32(
                _mm_blend_epi32(xt6, xt4, 0x02),
                xt2, 0x0C);
            rr!(xn0, xn1, xn2, xn3);

            // round 4
            let xt0 = _mm_shuffle_epi32(xn0, 0x80);
            let xt1 = _mm_shuffle_epi32(xn0, 0x4C);
            let xt2 = _mm_shuffle_epi32(xn1, 0x09);
            let xt3 = _mm_shuffle_epi32(xn1, 0x03);
            let xt4 = _mm_shuffle_epi32(xn2, 0x04);
            let xt5 = _mm_shuffle_epi32(xn3, 0x40);
            let xt6 = _mm_shuffle_epi32(xn3, 0x32);
            let xm0 = _mm_blend_epi32(
                _mm_blend_epi32(xn1, xt4, 0x06),
                xt5, 0x08);
            let xm1 = _mm_blend_epi32(
                _mm_blend_epi32(xt6, xt0, 0x02),
                xn2, 0x0C);
            let xm2 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt1, 0x0A),
                xt5, 0x04);
            let xm3 = _mm_blend_epi32(
                _mm_blend_epi32(xt2, xt6, 0x04),
                xt0, 0x08);
            rr!(xm0, xm1, xm2, xm3);

            // round 5
            let xt0 = _mm_shuffle_epi32(xm0, 0x04);
            let xt1 = _mm_shuffle_epi32(xm0, 0x0E);
            let xt2 = _mm_shuffle_epi32(xm1, 0x04);
            let xt3 = _mm_shuffle_epi32(xm1, 0x32);
            let xt4 = _mm_shuffle_epi32(xm2, 0x08);
            let xt5 = _mm_shuffle_epi32(xm2, 0xD0);
            let xt6 = _mm_shuffle_epi32(xm3, 0x01);
            let xt7 = _mm_shuffle_epi32(xm3, 0x83);
            let xn0 = _mm_blend_epi32(
                _mm_blend_epi32(xt1, xt4, 0x02),
                _mm_blend_epi32(xt2, xt7, 0x08), 0x0C);
            let xn1 = _mm_blend_epi32(
                _mm_blend_epi32(xt6, xt1, 0x02),
                xt5, 0x0C);
            let xn2 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt2, 0x02),
                xt6, 0x08);
            let xn3 = _mm_blend_epi32(
                _mm_blend_epi32(xt7, xt0, 0x0A),
                xt4, 0x04);
            rr!(xn0, xn1, xn2, xn3);

            // round 6
            let xt0 = _mm_shuffle_epi32(xn0, 0xC6);
            let xt1 = _mm_shuffle_epi32(xn1, 0x40);
            let xt2 = _mm_shuffle_epi32(xn1, 0x8C);
            let xt3 = _mm_shuffle_epi32(xn2, 0x09);
            let xt4 = _mm_shuffle_epi32(xn2, 0x0C);
            let xt5 = _mm_shuffle_epi32(xn3, 0x01);
            let xt6 = _mm_shuffle_epi32(xn3, 0x30);
            let xm0 = _mm_blend_epi32(
                _mm_blend_epi32(xt1, xt4, 0x0A),
                xn3, 0x04);
            let xm1 = _mm_blend_epi32(
                _mm_blend_epi32(xt5, xt3, 0x02),
                xt1, 0x08);
            let xm2 = _mm_blend_epi32(xt0, xt6, 0x04);
            let xm3 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt2, 0x0A),
                xt0, 0x04);
            rr!(xm0, xm1, xm2, xm3);

            // round 7
            let xt0 = _mm_shuffle_epi32(xm0, 0x0C);
            let xt1 = _mm_shuffle_epi32(xm0, 0x18);
            let xt2 = _mm_shuffle_epi32(xm1, 0xC2);
            let xt3 = _mm_shuffle_epi32(xm2, 0x10);
            let xt4 = _mm_shuffle_epi32(xm2, 0xB0);
            let xt5 = _mm_shuffle_epi32(xm3, 0x40);
            let xt6 = _mm_shuffle_epi32(xm3, 0x83);
            let xn0 = _mm_blend_epi32(
                _mm_blend_epi32(xt2, xt5, 0x0A),
                xt0, 0x04);
            let xn1 = _mm_blend_epi32(
                _mm_blend_epi32(xt6, xt1, 0x06),
                xt4, 0x08);
            let xn2 = _mm_blend_epi32(
                _mm_blend_epi32(xm1, xt4, 0x04),
                xt6, 0x08);
            let xn3 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt0, 0x02),
                xt2, 0x08);
            rr!(xn0, xn1, xn2, xn3);

            // round 8
            let xt0 = _mm_shuffle_epi32(xn0, 0x02);
            let xt1 = _mm_shuffle_epi32(xn0, 0x34);
            let xt2 = _mm_shuffle_epi32(xn1, 0x0C);
            let xt3 = _mm_shuffle_epi32(xn2, 0x03);
            let xt4 = _mm_shuffle_epi32(xn2, 0x81);
            let xt5 = _mm_shuffle_epi32(xn3, 0x02);
            let xt6 = _mm_shuffle_epi32(xn3, 0xD0);
            let xm0 = _mm_blend_epi32(
                _mm_blend_epi32(xt5, xn1, 0x02),
                xt2, 0x04);
            let xm1 = _mm_blend_epi32(
                _mm_blend_epi32(xt4, xt2, 0x02),
                xt1, 0x04);
            let xm2 = _mm_blend_epi32(
                _mm_blend_epi32(xt0, xn1, 0x04),
                xt6, 0x08);
            let xm3 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt1, 0x02),
                xt6, 0x04);
            rr!(xm0, xm1, xm2, xm3);

            // round 9
            let xt0 = _mm_shuffle_epi32(xm0, 0xC6);
            let xt1 = _mm_shuffle_epi32(xm1, 0x2C);
            let xt2 = _mm_shuffle_epi32(xm2, 0x40);
            let xt3 = _mm_shuffle_epi32(xm2, 0x83);
            let xt4 = _mm_shuffle_epi32(xm3, 0xD8);
            let xn0 = _mm_blend_epi32(
                _mm_blend_epi32(xt3, xt1, 0x02),
                xt4, 0x04);
            let xn1 = _mm_blend_epi32(xt4, xt0, 0x04);
            let xn2 = _mm_blend_epi32(
                _mm_blend_epi32(xm1, xt1, 0x04),
                xt2, 0x08);
            let xn3 = _mm_blend_epi32(xt0, xt2, 0x04);
            rr!(xn0, xn1, xn2, xn3);

            let xh0 = _mm_xor_si128(xh0, _mm_xor_si128(xv0, xv2));
            let xh1 = _mm_xor_si128(xh1, _mm_xor_si128(xv1, xv3));
            _mm_storeu_si128(transmute(&h[0]), xh0);
            _mm_storeu_si128(transmute(&h[4]), xh1);
        }

        #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
        unsafe {
            // x86_64, using SSE2.
            // Contrary to the AVX2 version, we do not have _mm_shuffle_epi8()
            // nor _mm_blend_epi32().
            use core::arch::x86_64::*;
            use core::mem::transmute;

            // Initialize state.
            let xh0 = _mm_loadu_si128(transmute(&h[0]));
            let xh1 = _mm_loadu_si128(transmute(&h[4]));
            let mut xv0 = xh0;
            let mut xv1 = xh1;
            let mut xv2 = _mm_loadu_si128(transmute(&Self::IV[0]));
            let mut xv3 = _mm_loadu_si128(transmute(&Self::IV[4]));
            xv3 = _mm_xor_si128(xv3, _mm_setr_epi32(
                ctr as i32, (ctr >> 32) as i32, -(last as i32), 0));

            // Load data and move it into the proper order for the first round:
            //   xm0:  0  2  4  6
            //   xm1:  1  3  5  7
            //   xm2:  8 10 12 14
            //   xm3:  9 11 13 15
            let xm0 = _mm_loadu_si128(transmute(&block[ 0]));
            let xm1 = _mm_loadu_si128(transmute(&block[16]));
            let xm2 = _mm_loadu_si128(transmute(&block[32]));
            let xm3 = _mm_loadu_si128(transmute(&block[48]));

            let xn0 = _mm_shuffle_epi32(xm0, 0xD8);
            let xn1 = _mm_shuffle_epi32(xm1, 0xD8);
            let xm0 = _mm_unpacklo_epi64(xn0, xn1);
            let xm1 = _mm_unpackhi_epi64(xn0, xn1);

            let xn2 = _mm_shuffle_epi32(xm2, 0xD8);
            let xn3 = _mm_shuffle_epi32(xm3, 0xD8);
            let xm2 = _mm_unpacklo_epi64(xn2, xn3);
            let xm3 = _mm_unpackhi_epi64(xn2, xn3);

            macro_rules! g4 { ($xx: expr, $xy: expr) => {
                xv0 = _mm_add_epi32(xv0, _mm_add_epi32(xv1, $xx));
                let xtg = _mm_xor_si128(xv0, xv3);
                xv3 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 16), _mm_slli_epi32(xtg, 16));
                xv2 = _mm_add_epi32(xv2, xv3);
                let xtg = _mm_xor_si128(xv1, xv2);
                xv1 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 12), _mm_slli_epi32(xtg, 20));
                xv0 = _mm_add_epi32(xv0, _mm_add_epi32(xv1, $xy));
                let xtg = _mm_xor_si128(xv0, xv3);
                xv3 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 8), _mm_slli_epi32(xtg, 24));
                xv2 = _mm_add_epi32(xv2, xv3);
                let xtg = _mm_xor_si128(xv1, xv2);
                xv1 = _mm_or_si128(
                    _mm_srli_epi32(xtg, 7), _mm_slli_epi32(xtg, 25));
            } }

            macro_rules! rr { ($i0: expr, $i1: expr, $i2: expr, $i3: expr) => {
                g4!($i0, $i1);
                xv1 = _mm_shuffle_epi32(xv1, 0x39);
                xv2 = _mm_shuffle_epi32(xv2, 0x4E);
                xv3 = _mm_shuffle_epi32(xv3, 0x93);
                g4!($i2, $i3);
                xv1 = _mm_shuffle_epi32(xv1, 0x93);
                xv2 = _mm_shuffle_epi32(xv2, 0x4E);
                xv3 = _mm_shuffle_epi32(xv3, 0x39);
            } }

            let xz1 = _mm_setr_epi32(-1, 0, 0, 0);
            let xz2 = _mm_setr_epi32(0, -1, 0, 0);
            let xz3 = _mm_setr_epi32(-1, -1, 0, 0);
            let xz4 = _mm_setr_epi32(0, 0, -1, 0);
            let xz5 = _mm_setr_epi32(-1, 0, -1, 0);
            let xz6 = _mm_setr_epi32(0, -1, -1, 0);
            let xz7 = _mm_setr_epi32(-1, -1, -1, 0);

            // round 0
            rr!(xm0, xm1, xm2, xm3);

            // round 1
            let xt0 = _mm_shuffle_epi32(xm0, 0x00);
            let xt1 = _mm_shuffle_epi32(xm0, 0xC8);
            let xt2 = _mm_shuffle_epi32(xm1, 0x70);
            let xt3 = _mm_shuffle_epi32(xm1, 0x80);
            let xt4 = _mm_shuffle_epi32(xm2, 0x01);
            let xt5 = _mm_shuffle_epi32(xm2, 0x02);
            let xt6 = _mm_shuffle_epi32(xm2, 0x03);
            let xt7 = _mm_shuffle_epi32(xm3, 0x80);
            let xt8 = _mm_shuffle_epi32(xm3, 0x10);
            let xt9 = _mm_shuffle_epi32(xm3, 0x30);
            let xn0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt6), _mm_and_si128(xz2, xt1)),
                _mm_andnot_si128(xz3, xt7));
            let xn1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xt4), _mm_and_si128(xz4, xt9)),
                _mm_andnot_si128(xz7, xt1));
            let xn2 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt3), _mm_and_si128(xz2, xt0)),
                _mm_and_si128(xz4, xt8));
            let xn3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt5), _mm_and_si128(xz2, xm0)),
                _mm_andnot_si128(xz3, xt2));
            rr!(xn0, xn1, xn2, xn3);

            // round 2
            let xt0 = _mm_shuffle_epi32(xn0, 0x40);
            let xt1 = _mm_shuffle_epi32(xn0, 0x80);
            let xt2 = _mm_shuffle_epi32(xn1, 0x80);
            let xt3 = _mm_shuffle_epi32(xn1, 0x0D);
            let xt4 = _mm_shuffle_epi32(xn2, 0x04);
            let xt5 = _mm_shuffle_epi32(xn2, 0x32);
            let xt6 = _mm_shuffle_epi32(xn3, 0x10);
            let xt7 = _mm_shuffle_epi32(xn3, 0x2C);
            let xm0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz5, xt5), _mm_and_si128(xz2, xt6)),
                _mm_andnot_si128(xz7, xt2));
            let xm1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt3), _mm_and_si128(xz2, xt4)),
                _mm_or_si128(_mm_and_si128(xz4, xt6), _mm_andnot_si128(xz7, xn0)));
            let xm2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt2), _mm_and_si128(xz6, xt7)),
                _mm_andnot_si128(xz7, xt1));
            let xm3 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt0), _mm_and_si128(xz2, xt3)),
                _mm_and_si128(xz4, xt4));
            rr!(xm0, xm1, xm2, xm3);

            // round 3
            let xt0 = _mm_shuffle_epi32(xm0, 0x10);
            let xt1 = _mm_shuffle_epi32(xm0, 0xC8);
            let xt2 = _mm_shuffle_epi32(xm1, 0x10);
            let xt3 = _mm_shuffle_epi32(xm1, 0x32);
            let xt4 = _mm_shuffle_epi32(xm2, 0x03);
            let xt5 = _mm_shuffle_epi32(xm2, 0x06);
            let xt6 = _mm_shuffle_epi32(xm3, 0x39);
            let xn0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xt5), _mm_and_si128(xz4, xt3)),
                _mm_andnot_si128(xz7, xt0));
            let xn1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt4), _mm_andnot_si128(xz5, xt6)),
                _mm_and_si128(xz4, xt0));
            let xn2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt3), _mm_andnot_si128(xz5, xt1)),
                _mm_and_si128(xz4, xt6));
            let xn3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt6), _mm_and_si128(xz2, xt4)),
                _mm_andnot_si128(xz3, xt2));
            rr!(xn0, xn1, xn2, xn3);

            // round 4
            let xt0 = _mm_shuffle_epi32(xn0, 0x80);
            let xt1 = _mm_shuffle_epi32(xn0, 0x4C);
            let xt2 = _mm_shuffle_epi32(xn1, 0x09);
            let xt3 = _mm_shuffle_epi32(xn1, 0x03);
            let xt4 = _mm_shuffle_epi32(xn2, 0x04);
            let xt5 = _mm_shuffle_epi32(xn3, 0x40);
            let xt6 = _mm_shuffle_epi32(xn3, 0x32);
            let xm0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xn1), _mm_and_si128(xz6, xt4)),
                _mm_andnot_si128(xz7, xt5));
            let xm1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt6), _mm_and_si128(xz2, xt0)),
                _mm_andnot_si128(xz3, xn2));
            let xm2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt3), _mm_andnot_si128(xz5, xt1)),
                _mm_and_si128(xz4, xt5));
            let xm3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xt2), _mm_and_si128(xz4, xt6)),
                _mm_andnot_si128(xz7, xt0));
            rr!(xm0, xm1, xm2, xm3);

            // round 5
            let xt0 = _mm_shuffle_epi32(xm0, 0x04);
            let xt1 = _mm_shuffle_epi32(xm0, 0x0E);
            let xt2 = _mm_shuffle_epi32(xm1, 0x04);
            let xt3 = _mm_shuffle_epi32(xm1, 0x32);
            let xt4 = _mm_shuffle_epi32(xm2, 0x08);
            let xt5 = _mm_shuffle_epi32(xm2, 0xD0);
            let xt6 = _mm_shuffle_epi32(xm3, 0x01);
            let xt7 = _mm_shuffle_epi32(xm3, 0x83);
            let xn0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt1), _mm_and_si128(xz2, xt4)),
                _mm_or_si128(_mm_and_si128(xz4, xt2), _mm_andnot_si128(xz7, xt7)));
            let xn1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt6), _mm_and_si128(xz2, xt1)),
                _mm_andnot_si128(xz3, xt5));
            let xn2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz5, xt3), _mm_and_si128(xz2, xt2)),
                _mm_andnot_si128(xz7, xt6));
            let xn3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt7), _mm_andnot_si128(xz5, xt0)),
                _mm_and_si128(xz4, xt4));
            rr!(xn0, xn1, xn2, xn3);

            // round 6
            let xt0 = _mm_shuffle_epi32(xn0, 0xC6);
            let xt1 = _mm_shuffle_epi32(xn1, 0x40);
            let xt2 = _mm_shuffle_epi32(xn1, 0x8C);
            let xt3 = _mm_shuffle_epi32(xn2, 0x09);
            let xt4 = _mm_shuffle_epi32(xn2, 0x0C);
            let xt5 = _mm_shuffle_epi32(xn3, 0x01);
            let xt6 = _mm_shuffle_epi32(xn3, 0x30);
            let xm0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt1), _mm_andnot_si128(xz5, xt4)),
                _mm_and_si128(xz4, xn3));
            let xm1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz5, xt5), _mm_and_si128(xz2, xt3)),
                _mm_andnot_si128(xz7, xt1));
            let xm2 = _mm_or_si128(_mm_andnot_si128(xz4, xt0), _mm_and_si128(xz4, xt6));
            let xm3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt3), _mm_andnot_si128(xz5, xt2)),
                _mm_and_si128(xz4, xt0));
            rr!(xm0, xm1, xm2, xm3);

            // round 7
            let xt0 = _mm_shuffle_epi32(xm0, 0x0C);
            let xt1 = _mm_shuffle_epi32(xm0, 0x18);
            let xt2 = _mm_shuffle_epi32(xm1, 0xC2);
            let xt3 = _mm_shuffle_epi32(xm2, 0x10);
            let xt4 = _mm_shuffle_epi32(xm2, 0xB0);
            let xt5 = _mm_shuffle_epi32(xm3, 0x40);
            let xt6 = _mm_shuffle_epi32(xm3, 0x83);
            let xn0 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt2), _mm_andnot_si128(xz5, xt5)),
                _mm_and_si128(xz4, xt0));
            let xn1 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz1, xt6), _mm_and_si128(xz6, xt1)),
                _mm_andnot_si128(xz7, xt4));
            let xn2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xm1), _mm_and_si128(xz4, xt4)),
                _mm_andnot_si128(xz7, xt6));
            let xn3 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz5, xt3), _mm_and_si128(xz2, xt0)),
                _mm_andnot_si128(xz7, xt2));
            rr!(xn0, xn1, xn2, xn3);

            // round 8
            let xt0 = _mm_shuffle_epi32(xn0, 0x02);
            let xt1 = _mm_shuffle_epi32(xn0, 0x34);
            let xt2 = _mm_shuffle_epi32(xn1, 0x0C);
            let xt3 = _mm_shuffle_epi32(xn2, 0x03);
            let xt4 = _mm_shuffle_epi32(xn2, 0x81);
            let xt5 = _mm_shuffle_epi32(xn3, 0x02);
            let xt6 = _mm_shuffle_epi32(xn3, 0xD0);
            let xm0 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt5), _mm_and_si128(xz2, xn1)),
                _mm_and_si128(xz4, xt2));
            let xm1 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt4), _mm_and_si128(xz2, xt2)),
                _mm_and_si128(xz4, xt1));
            let xm2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xt0), _mm_and_si128(xz4, xn1)),
                _mm_andnot_si128(xz7, xt6));
            let xm3 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt3), _mm_and_si128(xz2, xt1)),
                _mm_and_si128(xz4, xt6));
            rr!(xm0, xm1, xm2, xm3);

            // round 9
            let xt0 = _mm_shuffle_epi32(xm0, 0xC6);
            let xt1 = _mm_shuffle_epi32(xm1, 0x2C);
            let xt2 = _mm_shuffle_epi32(xm2, 0x40);
            let xt3 = _mm_shuffle_epi32(xm2, 0x83);
            let xt4 = _mm_shuffle_epi32(xm3, 0xD8);
            let xn0 = _mm_or_si128(
                _mm_or_si128(_mm_andnot_si128(xz6, xt3), _mm_and_si128(xz2, xt1)),
                _mm_and_si128(xz4, xt4));
            let xn1 = _mm_or_si128(_mm_andnot_si128(xz4, xt4), _mm_and_si128(xz4, xt0));
            let xn2 = _mm_or_si128(
                _mm_or_si128(_mm_and_si128(xz3, xm1), _mm_and_si128(xz4, xt1)),
                _mm_andnot_si128(xz7, xt2));
            let xn3 = _mm_or_si128(_mm_andnot_si128(xz4, xt0), _mm_and_si128(xz4, xt2));
            rr!(xn0, xn1, xn2, xn3);

            let xh0 = _mm_xor_si128(xh0, _mm_xor_si128(xv0, xv2));
            let xh1 = _mm_xor_si128(xh1, _mm_xor_si128(xv1, xv3));
            _mm_storeu_si128(transmute(&h[0]), xh0);
            _mm_storeu_si128(transmute(&h[4]), xh1);
        }
    }
}

#[cfg(test)]
mod tests {

    use super::{Blake2s256, Blake2s, KeyedBlake2s};

    static KAT_BLAKE2S: [[&str; 3]; 257] = [
        // Each group of three values is:
        //   input
        //   key
        //   output
        // All values are in hexadecimal. All outputs are 32-byte in length.
        //
        // First test vector is from RFC 7693.
        // Other vectors are from: https://github.com/BLAKE2/BLAKE2/
        [
            "616263",
            "",
            "508c5e8c327c14e2e1a72ba34eeb452f37458b209ed63a294d999b4c86675982",
        ], [
            "",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "48a8997da407876b3d79c0d92325ad3b89cbb754d86ab71aee047ad345fd2c49",
        ], [
            "00",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "40d15fee7c328830166ac3f918650f807e7e01e177258cdc0a39b11f598066f1",
        ], [
            "0001",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "6bb71300644cd3991b26ccd4d274acd1adeab8b1d7914546c1198bbe9fc9d803",
        ], [
            "000102",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1d220dbe2ee134661fdf6d9e74b41704710556f2f6e5a091b227697445dbea6b",
        ], [
            "00010203",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f6c3fbadb4cc687a0064a5be6e791bec63b868ad62fba61b3757ef9ca52e05b2",
        ], [
            "0001020304",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "49c1f21188dfd769aea0e911dd6b41f14dab109d2b85977aa3088b5c707e8598",
        ], [
            "000102030405",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "fdd8993dcd43f696d44f3cea0ff35345234ec8ee083eb3cada017c7f78c17143",
        ], [
            "00010203040506",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e6c8125637438d0905b749f46560ac89fd471cf8692e28fab982f73f019b83a9",
        ], [
            "0001020304050607",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "19fc8ca6979d60e6edd3b4541e2f967ced740df6ec1eaebbfe813832e96b2974",
        ], [
            "000102030405060708",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a6ad777ce881b52bb5a4421ab6cdd2dfba13e963652d4d6d122aee46548c14a7",
        ], [
            "00010203040506070809",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f5c4b2ba1a00781b13aba0425242c69cb1552f3f71a9a3bb22b4a6b4277b46dd",
        ], [
            "000102030405060708090a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e33c4c9bd0cc7e45c80e65c77fa5997fec7002738541509e68a9423891e822a3",
        ], [
            "000102030405060708090a0b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "fba16169b2c3ee105be6e1e650e5cbf40746b6753d036ab55179014ad7ef6651",
        ], [
            "000102030405060708090a0b0c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f5c4bec6d62fc608bf41cc115f16d61c7efd3ff6c65692bbe0afffb1fede7475",
        ], [
            "000102030405060708090a0b0c0d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a4862e76db847f05ba17ede5da4e7f91b5925cf1ad4ba12732c3995742a5cd6e",
        ], [
            "000102030405060708090a0b0c0d0e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "65f4b860cd15b38ef814a1a804314a55be953caa65fd758ad989ff34a41c1eea",
        ], [
            "000102030405060708090a0b0c0d0e0f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "19ba234f0a4f38637d1839f9d9f76ad91c8522307143c97d5f93f69274cec9a7",
        ], [
            "000102030405060708090a0b0c0d0e0f10",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1a67186ca4a5cb8e65fca0e2ecbc5ddc14ae381bb8bffeb9e0a103449e3ef03c",
        ], [
            "000102030405060708090a0b0c0d0e0f1011",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "afbea317b5a2e89c0bd90ccf5d7fd0ed57fe585e4be3271b0a6bf0f5786b0f26",
        ], [
            "000102030405060708090a0b0c0d0e0f101112",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f1b01558ce541262f5ec34299d6fb4090009e3434be2f49105cf46af4d2d4124",
        ], [
            "000102030405060708090a0b0c0d0e0f10111213",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "13a0a0c86335635eaa74ca2d5d488c797bbb4f47dc07105015ed6a1f3309efce",
        ], [
            "000102030405060708090a0b0c0d0e0f1011121314",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1580afeebebb346f94d59fe62da0b79237ead7b1491f5667a90e45edf6ca8b03",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "20be1a875b38c573dd7faaa0de489d655c11efb6a552698e07a2d331b5f655c3",
        ], [
            "000102030405060708090a0b0c0d0e0f10111213141516",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "be1fe3c4c04018c54c4a0f6b9a2ed3c53abe3a9f76b4d26de56fc9ae95059a99",
        ], [
            "000102030405060708090a0b0c0d0e0f1011121314151617",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e3e3ace537eb3edd8463d9ad3582e13cf86533ffde43d668dd2e93bbdbd7195a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "110c50c0bf2c6e7aeb7e435d92d132ab6655168e78a2decdec3330777684d9c1",
        ], [
            "000102030405060708090a0b0c0d0e0f10111213141516171819",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e9ba8f505c9c80c08666a701f3367e6cc665f34b22e73c3c0417eb1c2206082f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "26cd66fca02379c76df12317052bcafd6cd8c3a7b890d805f36c49989782433a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "213f3596d6e3a5d0e9932cd2159146015e2abc949f4729ee2632fe1edb78d337",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1015d70108e03be1c702fe97253607d14aee591f2413ea6787427b6459ff219a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3ca989de10cfe609909472c8d35610805b2f977734cf652cc64b3bfc882d5d89",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b6156f72d380ee9ea6acd190464f2307a5c179ef01fd71f99f2d0f7a57360aea",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c03bc642b20959cbe133a0303e0c1abff3e31ec8e1a328ec8565c36decff5265",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2c3e08176f760c6264c3a2cd66fec6c3d78de43fc192457b2a4a660a1e0eb22b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f738c02f3c1b190c512b1a32deabf353728e0e9ab034490e3c3409946a97aeec",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8b1880df301cc963418811088964839287ff7fe31c49ea6ebd9e48bdeee497c5",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1e75cb21c60989020375f1a7a242839f0b0b68973a4c2a05cf7555ed5aaec4c1",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021222324",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "62bf8a9c32a5bccf290b6c474d75b2a2a4093f1a9e27139433a8f2b3bce7b8d7",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "166c8350d3173b5e702b783dfd33c66ee0432742e9b92b997fd23c60dc6756ca",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223242526",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "044a14d822a90cacf2f5a101428adc8f4109386ccb158bf905c8618b8ee24ec3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021222324252627",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "387d397ea43a994be84d2d544afbe481a2000f55252696bba2c50c8ebd101347",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "56f8ccf1f86409b46ce36166ae9165138441577589db08cbc5f66ca29743b9fd",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20212223242526272829",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9706c092b04d91f53dff91fa37b7493d28b576b5d710469df79401662236fc03",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "877968686c068ce2f7e2adcff68bf8748edf3cf862cfb4d3947a3106958054e3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8817e5719879acf7024787eccdb271035566cfa333e049407c0178ccc57a5b9f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8938249e4b50cadaccdf5b18621326cbb15253e33a20f5636e995d72478de472",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f164abba4963a44d107257e3232d90aca5e66a1408248c51741e991db5227756",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d05563e2b1cba0c4a2a1e8bde3a1a0d9f5b40c85a070d6f5fb21066ead5d0601",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "03fbb16384f0a3866f4c3117877666efbf124597564b293d4aab0d269fabddfa",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f30",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5fa8486ac0e52964d1881bbe338eb54be2f719549224892057b4da04ba8b3475",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f3031",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "cdfabcee46911111236a31708b2539d71fc211d9b09c0d8530a11e1dbf6eed01",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4f82de03b9504793b82a07a0bdcdff314d759e7b62d26b784946b0d36f916f52",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f30313233",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "259ec7f173bcc76a0994c967b4f5f024c56057fb79c965c4fae41875f06a0e4c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f3031323334",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "193cc8e7c3e08bb30f5437aa27ade1f142369b246a675b2383e6da9b49a9809e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5c10896f0e2856b2a2eee0fe4a2c1633565d18f0e93e1fab26c373e8f829654d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f30313233343536",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f16012d93f28851a1eb989f5d0b43f3f39ca73c9a62d5181bff237536bd348c3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f3031323334353637",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2966b3cfae1e44ea996dc5d686cf25fa053fb6f67201b9e46eade85d0ad6b806",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ddb8782485e900bc60bcf4c33a6fd585680cc683d516efa03eb9985fad8715fb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f30313233343536373839",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4c4d6e71aea05786413148fc7a786b0ecaf582cff1209f5a809fba8504ce662c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "fb4c5e86d7b2229b99b8ba6d94c247ef964aa3a2bae8edc77569f28dbbff2d4e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e94f526de9019633ecd54ac6120f23958d7718f1e7717bf329211a4faeed4e6d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "cbd6660a10db3f23f7a03d4b9d4044c7932b2801ac89d60bc9eb92d65a46c2a0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8818bbd3db4dc123b25cbba5f54c2bc4b3fcf9bf7d7a7709f4ae588b267c4ece",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c65382513f07460da39833cb666c5ed82e61b9e998f4b0c4287cee56c3cc9bcd",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8975b0577fd35566d750b362b0897a26c399136df07bababbde6203ff2954ed4",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "21fe0ceb0052be7fb0f004187cacd7de67fa6eb0938d927677f2398c132317a8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f4041",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2ef73f3c26f12d93889f3c78b6a66c1d52b649dc9e856e2c172ea7c58ac2b5e3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "388a3cd56d73867abb5f8401492b6e2681eb69851e767fd84210a56076fb3dd3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40414243",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "af533e022fc9439e4e3cb838ecd18692232adf6fe9839526d3c3dd1b71910b1a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f4041424344",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "751c09d41a9343882a81cd13ee40818d12eb44c6c7f40df16e4aea8fab91972a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5b73ddb68d9d2b0aa265a07988d6b88ae9aac582af83032f8a9b21a2e1b7bf18",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40414243444546",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3da29126c7c5d7f43e64242a79feaa4ef3459cdeccc898ed59a97f6ec93b9dab",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f4041424344454647",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "566dc920293da5cb4fe0aa8abda8bbf56f552313bff19046641e3615c1e3ed3f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4115bea02f73f97f629e5c5590720c01e7e449ae2a6697d4d2783321303692f9",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40414243444546474849",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4ce08f4762468a7670012164878d68340c52a35e66c1884d5c864889abc96677",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "81ea0b7804124e0c22ea5fc71104a2afcb52a1fa816f3ecb7dcb5d9dea1786d0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "fe362733b05f6bedaf9379d7f7936ede209b1f8323c3922549d9e73681b5db7b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "eff37d30dfd20359be4e73fdf40d27734b3df90a97a55ed745297294ca85d09f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "172ffc67153d12e0ca76a8b6cd5d4731885b39ce0cac93a8972a18006c8b8baf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c47957f1cc88e83ef9445839709a480a036bed5f88ac0fcc8e1e703ffaac132c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "30f3548370cfdceda5c37b569b6175e799eef1a62aaa943245ae7669c227a7b5",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f50",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c95dcb3cf1f27d0eef2f25d2413870904a877c4a56c2de1e83e2bc2ae2e46821",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f5051",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d5d0b5d705434cd46b185749f66bfb5836dcdf6ee549a2b7a4aee7f58007caaf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "bbc124a712f15d07c300e05b668389a439c91777f721f8320c1c9078066d2c7e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f50515253",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a451b48c35a6c7854cfaae60262e76990816382ac0667e5a5c9e1b46c4342ddf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f5051525354",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b0d150fb55e778d01147f0b5d89d99ecb20ff07e5e6760d6b645eb5b654c622b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "34f737c0ab219951eee89a9f8dac299c9d4c38f33fa494c5c6eefc92b6db08bc",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f50515253545556",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1a62cc3a00800dcbd99891080c1e098458193a8cc9f970ea99fbeff00318c289",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f5051525354555657",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "cfce55ebafc840d7ae48281c7fd57ec8b482d4b704437495495ac414cf4a374b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "6746facf71146d999dabd05d093ae586648d1ee28e72617b99d0f0086e1e45bf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f50515253545556575859",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "571ced283b3f23b4e750bf12a2caf1781847bd890e43603cdc5976102b7bb11b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "cfcb765b048e35022c5d089d26e85a36b005a2b80493d03a144e09f409b6afd1",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4050c7a27705bb27f42089b299f3cbe5054ead68727e8ef9318ce6f25cd6f31d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "184070bd5d265fbdc142cd1c5cd0d7e414e70369a266d627c8fba84fa5e84c34",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9edda9a4443902a9588c0d0ccc62b930218479a6841e6fe7d43003f04b1fd643",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e412feef7908324a6da1841629f35d3d358642019310ec57c614836b63d30763",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1a2b8edff3f9acc1554fcbae3cf1d6298c6462e22e5eb0259684f835012bd13f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "288c4ad9b9409762ea07c24a41f04f69a7d74bee2d95435374bde946d7241c7b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f6061",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "805691bb286748cfb591d3aebe7e6f4e4dc6e2808c65143cc004e4eb6fd09d43",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d4ac8d3a0afc6cfa7b460ae3001baeb36dadb37da07d2e8ac91822df348aed3d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60616263",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c376617014d20158bced3d3ba552b6eccf84e62aa3eb650e90029c84d13eea69",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f6061626364",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c41f09f43cecae7293d6007ca0a357087d5ae59be500c1cd5b289ee810c7b082",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "03d1ced1fba5c39155c44b7765cb760c78708dcfc80b0bd8ade3a56da8830b29",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60616263646566",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "09bde6f152218dc92c41d7f45387e63e5869d807ec70b821405dbd884b7fcf4b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f6061626364656667",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "71c9036e18179b90b37d39e9f05eb89cc5fc341fd7c477d0d7493285faca08a4",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5916833ebb05cd919ca7fe83b692d3205bef72392b2cf6bb0a6d43f994f95f11",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60616263646566676869",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f63aab3ec641b3b024964c2b437c04f6043c4c7e0279239995401958f86bbe54",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f172b180bfb09740493120b6326cbdc561e477def9bbcfd28cc8c1c5e3379a31",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "cb9b89cc18381dd9141ade588654d4e6a231d5bf49d4d59ac27d869cbe100cf3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7bd8815046fdd810a923e1984aaebdcdf84d87c8992d68b5eeb460f93eb3c8d7",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "607be66862fd08ee5b19facac09dfdbcd40c312101d66e6ebd2b841f1b9a9325",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9fe03bbe69ab1834f5219b0da88a08b30a66c5913f0151963c360560db0387b3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "90a83585717b75f0e9b725e055eeeeb9e7a028ea7e6cbc07b20917ec0363e38c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f70",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "336ea0530f4a7469126e0218587ebbde3358a0b31c29d200f7dc7eb15c6aadd8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f7071",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a79e76dc0abca4396f0747cd7b748df913007626b1d659da0c1f78b9303d01a3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "44e78a773756e0951519504d7038d28d0213a37e0ce375371757bc996311e3b8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f70717273",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "77ac012a3f754dcfeab5eb996be9cd2d1f96111b6e49f3994df181f28569d825",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f7071727374",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ce5a10db6fccdaf140aaa4ded6250a9c06e9222bc9f9f3658a4aff935f2b9f3a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ecc203a7fe2be4abd55bb53e6e673572e0078da8cd375ef430cc97f9f80083af",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f70717273747576",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "14a5186de9d7a18b0412b8563e51cc5433840b4a129a8ff963b33a3c4afe8ebb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f7071727374757677",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "13f8ef95cb86e6a638931c8e107673eb76ba10d7c2cd70b9d9920bbeed929409",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "0b338f4ee12f2dfcb78713377941e0b0632152581d1332516e4a2cab1942cca4",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f70717273747576777879",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "eaab0ec37b3b8ab796e9f57238de14a264a076f3887d86e29bb5906db5a00e02",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "23cb68b8c0e6dc26dc27766ddc0a13a99438fd55617aa4095d8f969720c872df",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "091d8ee30d6f2968d46b687dd65292665742de0bb83dcc0004c72ce10007a549",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7f507abc6d19ba00c065a876ec5657868882d18a221bc46c7a6912541f5bc7ba",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a0607c24e14e8c223db0d70b4d30ee88014d603f437e9e02aa7dafa3cdfbad94",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ddbfea75cc467882eb3483ce5e2e756a4f4701b76b445519e89f22d60fa86e06",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "0c311f38c35a4fb90d651c289d486856cd1413df9b0677f53ece2cd9e477c60a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f80",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "46a73a8dd3e70f59d3942c01df599def783c9da82fd83222cd662b53dce7dbdf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ad038ff9b14de84a801e4e621ce5df029dd93520d0c2fa38bff176a8b1d1698c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ab70c5dfbd1ea817fed0cd067293abf319e5d7901c2141d5d99b23f03a38e748",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f80818283",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1fffda67932b73c8ecaf009a3491a026953babfe1f663b0697c3c4ae8b2e7dcb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b0d2cc19472dd57f2b17efc03c8d58c2283dbb19da572f7755855aa9794317a0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a0d19a6ee33979c325510e276622df41f71583d07501b87071129a0ad94732a5",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f80818283848586",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "724642a7032d1062b89e52bea34b75df7d8fe772d9fe3c93ddf3c4545ab5a99b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384858687",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ade5eaa7e61f672d587ea03dae7d7b55229c01d06bc0a5701436cbd18366a626",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "013b31ebd228fcdda51fabb03bb02d60ac20ca215aafa83bdd855e3755a35f0b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f80818283848586878889",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "332ed40bb10dde3c954a75d7b8999d4b26a1c063c1dc6e32c1d91bab7bbb7d16",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c7a197b3a05b566bcc9facd20e441d6f6c2860ac9651cd51d6b9d2cdeeea0390",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "bd9cf64ea8953c037108e6f654914f3958b68e29c16700dc184d94a21708ff60",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8835b0ac021151df716474ce27ce4d3c15f0b2dab48003cf3f3efd0945106b9a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3bfefa3301aa55c080190cffda8eae51d9af488b4c1f24c3d9a75242fd8ea01d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "08284d14993cd47d53ebaecf0df0478cc182c89c00e1859c84851686ddf2c1b7",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1ed7ef9f04c2ac8db6a864db131087f27065098e69c3fe78718d9b947f4a39d0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c161f2dcd57e9c1439b31a9dd43d8f3d7dd8f0eb7cfac6fb25a0f28e306f0661",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f9091",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c01969ad34c52caf3dc4d80d19735c29731ac6e7a92085ab9250c48dea48a3fc",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1720b3655619d2a52b3521ae0e49e345cb3389ebd6208acaf9f13fdacca8be49",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "756288361c83e24c617cf95c905b22d017cdc86f0bf1d658f4756c7379873b7f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f9091929394",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e7d0eda3452693b752abcda1b55e276f82698f5f1605403eff830bea0071a394",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2c82ecaa6b84803e044af63118afe544687cb6e6c7df49ed762dfd7c8693a1bc",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293949596",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "6136cbf4b441056fa1e2722498125d6ded45e17b52143959c7f4d4e395218ac2",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f9091929394959697",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "721d3245aafef27f6a624f47954b6c255079526ffa25e9ff77e5dcff473b1597",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9dd2fbd8cef16c353c0ac21191d509eb28dd9e3e0d8cea5d26ca839393851c3a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f90919293949596979899",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b2394ceacdebf21bf9df2ced98e58f1c3a4bbbff660dd900f62202d6785cc46e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "57089f222749ad7871765f062b114f43ba20ec56422a8b1e3f87192c0ea718c6",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e49a9459961cd33cdf4aae1b1078a5dea7c040e0fea340c93a724872fc4af806",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ede67f720effd2ca9c88994152d0201dee6b0a2d2c077aca6dae29f73f8b6309",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e0f434bf22e3088039c21f719ffc67f0f2cb5e98a7a0194c76e96bf4e8e17e61",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "277c04e2853484a4eba910ad336d01b477b67cc200c59f3c8d77eef8494f29cd",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "156d5747d0c99c7f27097d7b7e002b2e185cb72d8dd7eb424a0321528161219f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "20ddd1ed9b1ca803946d64a83ae4659da67fba7a1a3eddb1e103c0f5e03e3a2c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f0af604d3dabbf9a0f2a7d3dda6bd38bba72c6d09be494fcef713ff10189b6e6",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9802bb87def4cc10c4a5fd49aa58dfe2f3fddb46b4708814ead81d23ba95139b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4f8ce1e51d2fe7f24043a904d898ebfc91975418753413aa099b795ecb35cedb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "bddc6514d7ee6ace0a4ac1d0e068112288cbcf560454642705630177cba608bd",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d635994f6291517b0281ffdd496afa862712e5b3c4e52e4cd5fdae8c0e72fb08",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "878d9ca600cf87e769cc305c1b35255186615a73a0da613b5f1c98dbf81283ea",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a64ebe5dc185de9fdde7607b6998702eb23456184957307d2fa72e87a47702d6",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ce50eab7b5eb52bdc9ad8e5a480ab780ca9320e44360b1fe37e03f2f7ad7de01",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "eeddb7c0db6e30abe66d79e327511e61fcebbc29f159b40a86b046ecf0513823",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aa",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "787fc93440c1ec96b5ad01c16cf77916a1405f9426356ec921d8dff3ea63b7e0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaab",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7f0d5eab47eefda696c0bf0fbf86ab216fce461e9303aba6ac374120e890e8df",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabac",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b68004b42f14ad029f4c2e03b1d5eb76d57160e26476d21131bef20ada7d27f4",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacad",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b0c4eb18ae250b51a41382ead92d0dc7455f9379fc9884428e4770608db0faec",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadae",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f92b7a870c059f4d46464c824ec96355140bdce681322cc3a992ff103e3fea52",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5364312614813398cc525d4c4e146edeb371265fba19133a2c3d2159298a1742",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f6620e68d37fb2af5000fc28e23b832297ecd8bce99e8be4d04e85309e3d3374",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5316a27969d7fe04ff27b283961bffc3bf5dfb32fb6a89d101c6c3b1937c2871",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "81d1664fdf3cb33c24eebac0bd64244b77c4abea90bbe8b5ee0b2aafcf2d6a53",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "345782f295b0880352e924a0467b5fbc3e8f3bfbc3c7e48b67091fb5e80a9442",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "794111ea6cd65e311f74ee41d476cb632ce1e4b051dc1d9e9d061a19e1d0bb49",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2a85daf6138816b99bf8d08ba2114b7ab07975a78420c1a3b06a777c22dd8bcb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "89b0d5f289ec16401a069a960d0b093e625da3cf41ee29b59b930c5820145455",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d0fdcb543943fc27d20864f52181471b942cc77ca675bcb30df31d358ef7b1eb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b17ea8d77063c709d4dc6b879413c343e3790e9e62ca85b7900b086f6b75c672",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e71a3e2c274db842d92114f217e2c0eac8b45093fdfd9df4ca7162394862d501",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9ba",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c0476759ab7aa333234f6b44f5fd858390ec23694c622cb986e769c78edd733e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babb",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9ab8eabb1416434d85391341d56993c55458167d4418b19a0f2ad8b79a83a75b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbc",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7992d0bbb15e23826f443e00505d68d3ed7372995a5c3e498654102fbcd0964e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbd",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c021b30085151435df33b007ccecc69df1269f39ba25092bed59d932ac0fdc28",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbe",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "91a25ec0ec0d9a567f89c4bfe1a65a0e432d07064b4190e27dfb81901fd3139b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "5950d39a23e1545f301270aa1a12f2e6c453776e4d6355de425cc153f9818867",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d79f14720c610af179a3765d4b7c0968f977962dbf655b521272b6f1e194488e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e9531bfc8b02995aeaa75ba27031fadbcbf4a0dab8961d9296cd7e84d25d6006",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "34e9c26a01d7f16181b454a9d1623c233cb99d31c694656e9413aca3e918692f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d9d7422f437bd439ddd4d883dae2a08350173414be78155133fff1964c3d7972",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4aee0c7aaf075414ff1793ead7eaca601775c615dbd60b640b0a9f0ce505d435",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "6bfdd15459c83b99f096bfb49ee87b063d69c1974c6928acfcfb4099f8c4ef67",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9fd1c408fd75c336193a2a14d94f6af5adf050b80387b4b010fb29f4cc72707c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "13c88480a5d00d6c8c7ad2110d76a82d9b70f4fa6696d4e5dd42a066dcaf9920",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "820e725ee25fe8fd3a8d5abe4c46c3ba889de6fa9191aa22ba67d5705421542b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "32d93a0eb02f42fbbcaf2bad0085b282e46046a4df7ad10657c9d6476375b93e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9ca",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "adc5187905b1669cd8ec9c721e1953786b9d89a9bae30780f1e1eab24a00523c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacb",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "e90756ff7f9ad810b239a10ced2cf9b2284354c1f8c7e0accc2461dc796d6e89",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcc",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1251f76e56978481875359801db589a0b22f86d8d634dc04506f322ed78f17e8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccd",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3afa899fd980e73ecb7f4d8b8f291dc9af796bc65d27f974c6f193c9191a09fd",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdce",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "aa305be26e5deddc3c1010cbc213f95f051c785c5b431e6a7cd048f161787528",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8ea1884ff32e9d10f039b407d0d44e7e670abd884aeee0fb757ae94eaa97373d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d482b2155d4dec6b4736a1f1617b53aaa37310277d3fef0c37ad41768fc235b4",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "4d413971387e7a8898a8dc2a27500778539ea214a2dfe9b3d7e8ebdce5cf3db3",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "696e5d46e6c57e8796e4735d08916e0b7929b3cf298c296d22e9d3019653371c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1f5647c1d3b088228885865c8940908bf40d1a8272821973b160008e7a3ce2eb",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b6e76c330f021a5bda65875010b0edf09126c0f510ea849048192003aef4c61c",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3cd952a0beada41abb424ce47f94b42be64e1ffb0fd0782276807946d0d0bc55",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "98d92677439b41b7bb513312afb92bcc8ee968b2e3b238cecb9b0f34c9bb63d0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ecbca2cf08ae57d517ad16158a32bfa7dc0382eaeda128e91886734c24a0b29d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "942cc7c0b52e2b16a4b89fa4fc7e0bf609e29a08c1a8543452b77c7bfd11bb28",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8a065d8b61a0dffb170d5627735a76b0e9506037808cba16c345007c9f79cf8f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9da",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "1b9fa19714659c78ff413871849215361029ac802b1cbcd54e408bd87287f81f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadb",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8dab071bcd6c7292a9ef727b4ae0d86713301da8618d9a48adce55f303a869a1",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdc",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8253e3e7c7b684b9cb2beb014ce330ff3d99d17abbdbabe4f4d674ded53ffc6b",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdd",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "f195f321e9e3d6bd7d074504dd2ab0e6241f92e784b1aa271ff648b1cab6d7f6",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcddde",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "27e4cc72090f241266476a7c09495f2db153d5bcbd761903ef79275ec56b2ed8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "899c2405788e25b99a1846355e646d77cf400083415f7dc5afe69d6e17c00023",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a59b78c4905744076bfee894de707d4f120b5c6893ea0400297d0bb834727632",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "59dc78b105649707a2bb4419c48f005400d3973de3736610230435b10424b24f",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c0149d1d7e7a6353a6d906efe728f2f329fe14a4149a3ea77609bc42b975ddfa",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "a32f241474a6c16932e9243be0cf09bcdc7e0ca0e7a6a1b9b1a0f01e41502377",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b239b2e4f81841361c1339f68e2c359f929af9ad9f34e01aab4631ad6d5500b0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "85fb419c7002a3e0b4b6ea093b4c1ac6936645b65dac5ac15a8528b7b94c1754",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9619720625f190b93a3fad186ab314189633c0d3a01e6f9bc8c4a8f82f383dbf",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7d620d90fe69fa469a6538388970a1aa09bb48a2d59b347b97e8ce71f48c7f46",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "294383568596fb37c75bbacd979c5ff6f20a556bf8879cc72924855df9b8240e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "16b18ab314359c2b833c1c6986d48c55a9fc97cde9a3c1f10a3177140f73f738",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9ea",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8cbbdd14bc33f04cf45813e4a153a273d36adad5ce71f499eeb87fb8ac63b729",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaeb",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "69c9a498db174ecaefcc5a3ac9fdedf0f813a5bec727f1e775babdec7718816e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebec",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "b462c3be40448f1d4f80626254e535b08bc9cdcff599a768578d4b2881a8e3f0",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebeced",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "553e9d9c5f360ac0b74a7d44e5a391dad4ced03e0c24183b7e8ecabdf1715a64",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedee",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "7a7c55a56fa9ae51e655e01975d8a6ff4ae9e4b486fcbe4eac044588f245ebea",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "2afdf3c82abc4867f5de111286c2b3be7d6e48657ba923cfbf101a6dfcf9db9a",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "41037d2edcdce0c49b7fb4a6aa0999ca66976c7483afe631d4eda283144f6dfc",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c4466f8497ca2eeb4583a0b08e9d9ac74395709fda109d24f2e4462196779c5d",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "75f609338aa67d969a2ae2a2362b2da9d77c695dfd1df7224a6901db932c3364",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "68606ceb989d5488fc7cf649f3d7c272ef055da1a93faecd55fe06f6967098ca",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "44346bdeb7e052f6255048f0d9b42c425bab9c3dd24168212c3ecf1ebf34e6ae",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "8e9cf6e1f366471f2ac7d2ee9b5e6266fda71f8f2e4109f2237ed5f8813fc718",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "84bbeb8406d250951f8c1b3e86a7c010082921833dfd9555a2f909b1086eb4b8",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "ee666f3eef0f7e2a9c222958c97eaf35f51ced393d714485ab09a069340fdf88",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "c153d34a65c47b4a62c5cacf24010975d0356b2f32c8f5da530d338816ad5de6",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "9fc5450109e1b779f6c7ae79d56c27635c8dd426c5a9d54e2578db989b8c3b4e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fa",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "d12bf3732ef4af5c22fa90356af8fc50fcb40f8f2ea5c8594737a3b3d5abdbd7",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafb",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "11030b9289bba5af65260672ab6fee88b87420acef4a1789a2073b7ec2f2a09e",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfc",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "69cb192b8444005c8c0ceb12c846860768188cda0aec27a9c8a55cdee2123632",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfd",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "db444c15597b5f1a03d1f9edd16e4a9f43a667cc275175dfa2b704e3bb1a9b83",
        ], [
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfe",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "3fb735061abc519dfe979e54c1ee5bfad0a9d858b3315bad34bde999efd724dd",
        ],
    ];

    #[test]
    fn KAT() {
        for i in 0..KAT_BLAKE2S.len() {
            let data = hex::decode(KAT_BLAKE2S[i][0]).unwrap();
            let key = hex::decode(KAT_BLAKE2S[i][1]).unwrap();
            let refout = hex::decode(KAT_BLAKE2S[i][2]).unwrap();
            let out_len = refout.len();

            let mut sh = KeyedBlake2s::new(out_len, &key);
            let mut buf = [0u8; 32];
            sh.update(&data);
            assert!(out_len == sh.finalize_reset_write(&mut buf[..]));
            assert!(buf[..out_len] == refout[..]);
            for j in 0..data.len() {
                sh.update(&data[j..(j + 1)]);
            }
            assert!(out_len == sh.finalize_reset_write(&mut buf[..]));
            assert!(buf[..out_len] == refout[..]);
        }
    }

    #[test]
    fn rfc7693_selftest() {
        // We use the code from RFC 7693 (appendix E).

        fn selftest_seq(out: &mut [u8], seed: u32) {
            let mut a = seed.wrapping_mul(0xDEAD4BAD);
            let mut b = 1;
            for i in 0..out.len() {
                let t = a.wrapping_add(b);
                a = b;
                b = t;
                out[i] = (t >> 24) as u8;
            }
        }

        const BLAKE2S_RES: [u8; 32] = [
            0x6A, 0x41, 0x1F, 0x08, 0xCE, 0x25, 0xAD, 0xCD,
            0xFB, 0x02, 0xAB, 0xA6, 0x41, 0x45, 0x1C, 0xEC,
            0x53, 0xC5, 0x98, 0xB2, 0x4F, 0x4F, 0xC7, 0x87,
            0xFB, 0xDC, 0x88, 0x79, 0x7F, 0x4C, 0x1D, 0xFE,
        ];

        // Parameter sets.
        const B2S_MD_LEN: [usize; 4] = [ 16, 20, 28, 32 ];
        const B2S_IN_LEN: [usize; 6] = [ 0, 3, 64, 65, 255, 1024 ];

        let mut inbuf = [0u8; 1024];
        let mut md = [0u8; 32];
        let mut key = [0u8; 32];

        let mut ctx = Blake2s256::new();

        for i in 0..B2S_MD_LEN.len() {
            let outlen = B2S_MD_LEN[i];
            for j in 0..B2S_IN_LEN.len() {
                let inlen = B2S_IN_LEN[j];

                selftest_seq(&mut inbuf[..inlen], inlen as u32);
                Blake2s::hash_into(outlen, &inbuf[..inlen], &mut md);
                ctx.update(&md[..outlen]);

                selftest_seq(&mut key[..outlen], outlen as u32);
                KeyedBlake2s::hash_into(outlen, &key[..outlen],
                    &inbuf[..inlen], &mut md);
                ctx.update(&md[..outlen]);
            }
        }

        assert!(ctx.finalize() == BLAKE2S_RES);
    }
}
