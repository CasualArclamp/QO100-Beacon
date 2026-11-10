// made by CasualArclamp
//! The two pieces of CCSDS coding (131.0-B) that the beacon's FEC block
//! uses: the randomiser and the Reed–Solomon code. Taken unchanged from
//! DecSAT (`crates/decsat-modem/src/ccsds.rs`).
#![allow(dead_code)]

/// The randomiser's sequence (131.0-B §10.4.1), `n` bytes from the start of
/// a codeblock: FF 48 0E C0 9A 0D 70 BC … (period 255 bits). XORed over the
/// codeblock after the marker, at both ends.
pub fn randomiser(n: usize) -> Vec<u8> {
    // Taps of h(x) = x⁸ + x⁷ + x⁵ + x³ + 1 on a register shifted right; its
    // low bit is the output. All ones at the start (§10.4.2).
    let mut s: u32 = 0xFF;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let mut byte = 0u8;
        for _ in 0..8 {
            byte = (byte << 1) | (s & 1) as u8;
            let fb = (s ^ (s >> 3) ^ (s >> 5) ^ (s >> 7)) & 1;
            s = (s >> 1) | (fb << 7);
        }
        out.push(byte);
    }
    out
}

// ---------------------------------------------------------------------------
// Reed–Solomon over GF(256) with F(x) = x⁸ + x⁷ + x² + x + 1.

/// The field and the dual-basis conversion tables.
struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
    /// Conventional → Berlekamp dual basis (131.0-B Annex F), and back.
    to_dual: [u8; 256],
    from_dual: [u8; 256],
}

impl Gf {
    const fn new() -> Gf {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        let mut i = 0;
        while i < 255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x187;
            }
            i += 1;
        }
        while i < 512 {
            exp[i] = exp[i - 255];
            i += 1;
        }
        // The dual basis: the rows of 131.0-B Annex F's matrix T, applied to
        // the conventional bits (α⁷ … α⁰ as bits 7 … 0). Same construction
        // as Karn's `gen_ccsds_tab`: to_dual[1] = 7Bh, to_dual[2] = AFh, …
        let tal: [u8; 8] = [0x8d, 0xef, 0xec, 0x86, 0xfa, 0x99, 0xaf, 0x7b];
        let mut to_dual = [0u8; 256];
        let mut from_dual = [0u8; 256];
        let mut v = 0;
        while v < 256 {
            let mut d = 0u8;
            let mut k = 0;
            while k < 8 {
                if v & (1 << k) != 0 {
                    d ^= tal[7 - k];
                }
                k += 1;
            }
            to_dual[v] = d;
            from_dual[d as usize] = v as u8;
            v += 1;
        }
        Gf {
            exp,
            log,
            to_dual,
            from_dual,
        }
    }

    #[inline]
    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize]
        }
    }

    #[inline]
    fn div(&self, a: u8, b: u8) -> u8 {
        if a == 0 {
            0
        } else {
            self.exp[(self.log[a as usize] as usize + 255 - self.log[b as usize] as usize) % 255]
        }
    }

    /// α^i for any i ≥ 0.
    #[inline]
    fn pow(&self, i: usize) -> u8 {
        self.exp[i % 255]
    }
}

static GF: Gf = Gf::new();

/// β = α¹¹, the element whose powers the generator's roots are (§4.3.2).
const PRIM: usize = 11;

/// A codeword that could not be corrected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uncorrectable;

/// One CCSDS Reed–Solomon code, shortened to `n` symbols (virtual fill: the
/// leading 255 − n symbols are zeros, never sent).
#[derive(Debug, Clone)]
pub struct Rs {
    /// Correctable symbol errors: 16 for (255, 223), 8 for (255, 239).
    pub e: usize,
    /// Symbols sent per codeword (≤ 255).
    pub n: usize,
    /// Generator coefficients, conventional basis, highest degree first
    /// (g[0] = 1).
    g: Vec<u8>,
}

impl Rs {
    /// The E-error-correcting code shortened to `n` symbols.
    ///
    /// # Panics
    /// If E is not 8 or 16, or `n` leaves no data symbols.
    pub fn new(e: usize, n: usize) -> Self {
        assert!(e == 8 || e == 16, "CCSDS RS has E = 8 or 16");
        assert!(n <= 255 && n > 2 * e);
        // g(x) = Π (x − β^j), j = 128 − E … 127 + E (§4.3.2).
        let mut g = vec![1u8];
        for j in (128 - e)..(128 + e) {
            let root = GF.pow(PRIM * j);
            let mut next = vec![0u8; g.len() + 1];
            for (i, &c) in g.iter().enumerate() {
                next[i] ^= c;
                next[i + 1] ^= GF.mul(c, root);
            }
            g = next;
        }
        Rs { e, n, g }
    }

    /// Data symbols a codeword carries.
    pub fn k(&self) -> usize {
        self.n - 2 * self.e
    }

    /// Fill in the parity after the `k` data symbols of `cw` (`n` long).
    /// `dual`: the symbols are in the dual basis, as sent.
    pub fn encode(&self, cw: &mut [u8], dual: bool) {
        assert_eq!(cw.len(), self.n);
        let p = 2 * self.e;
        let k = self.k();
        let mut rem = vec![0u8; p];
        for &d in &cw[..k] {
            let m = if dual { GF.from_dual[d as usize] } else { d };
            let fb = m ^ rem[0];
            rem.rotate_left(1);
            rem[p - 1] = 0;
            if fb != 0 {
                for (r, &g) in rem.iter_mut().zip(&self.g[1..]) {
                    *r ^= GF.mul(fb, g);
                }
            }
        }
        for (c, r) in cw[k..].iter_mut().zip(rem) {
            *c = if dual { GF.to_dual[r as usize] } else { r };
        }
    }

    /// Correct `cw` in place (`n` symbols); returns the symbols corrected.
    pub fn decode(&self, cw: &mut [u8], dual: bool) -> Result<usize, Uncorrectable> {
        assert_eq!(cw.len(), self.n);
        let p = 2 * self.e;
        let fcr = 128 - self.e;
        let conv: Vec<u8> = if dual {
            cw.iter().map(|&b| GF.from_dual[b as usize]).collect()
        } else {
            cw.to_vec()
        };
        // Syndromes S_i = r(β^(fcr + i)); symbol 0 of the shortened word is
        // the coefficient of x^(n − 1).
        let mut s = vec![0u8; p];
        let mut clean = true;
        for (i, si) in s.iter_mut().enumerate() {
            let x = GF.pow(PRIM * (fcr + i));
            let mut acc = 0u8;
            for &c in &conv {
                acc = GF.mul(acc, x) ^ c;
            }
            *si = acc;
            clean &= acc == 0;
        }
        if clean {
            return Ok(0);
        }
        // Berlekamp–Massey: Λ(x) = Π (1 − X_l x), coefficients lowest first.
        let mut lambda = vec![0u8; p + 1];
        lambda[0] = 1;
        let mut b = lambda.clone();
        let mut l = 0usize;
        let mut m = 1usize;
        let mut bb = 1u8;
        for r in 0..p {
            let mut d = s[r];
            for i in 1..=l {
                d ^= GF.mul(lambda[i], s[r - i]);
            }
            if d == 0 {
                m += 1;
            } else if 2 * l <= r {
                let t = lambda.clone();
                let coef = GF.div(d, bb);
                for i in m..=p {
                    lambda[i] ^= GF.mul(coef, b[i - m]);
                }
                l = r + 1 - l;
                b = t;
                bb = d;
                m = 1;
            } else {
                let coef = GF.div(d, bb);
                for i in m..=p {
                    lambda[i] ^= GF.mul(coef, b[i - m]);
                }
                m += 1;
            }
        }
        if l > self.e {
            return Err(Uncorrectable);
        }
        // Chien search over the degrees actually sent: an error on x^e has
        // X = β^e, a root of Λ at β^(−e).
        let mut locs = Vec::with_capacity(l);
        for e in 0..self.n {
            let xinv = GF.pow(PRIM * (255 - e % 255));
            let mut acc = 0u8;
            let mut xp = 1u8;
            for &c in lambda.iter().take(l + 1) {
                acc ^= GF.mul(c, xp);
                xp = GF.mul(xp, xinv);
            }
            if acc == 0 {
                locs.push(e);
            }
        }
        if locs.len() != l {
            return Err(Uncorrectable);
        }
        // Ω(x) = S(x)·Λ(x) mod x^p.
        let mut omega = vec![0u8; p];
        for (i, o) in omega.iter_mut().enumerate() {
            let mut acc = 0u8;
            for j in 0..=i.min(l) {
                acc ^= GF.mul(lambda[j], s[i - j]);
            }
            *o = acc;
        }
        // Forney, for first consecutive root fcr:
        // Y = X^(1 − fcr) · Ω(X⁻¹) / Λ'(X⁻¹).
        let mut fixed = conv;
        for &e in &locs {
            let x = GF.pow(PRIM * e);
            let xinv = GF.pow(PRIM * (255 - e % 255));
            let mut num = 0u8;
            let mut xp = 1u8;
            for &o in &omega {
                num ^= GF.mul(o, xp);
                xp = GF.mul(xp, xinv);
            }
            // Λ'(x): the odd terms, i·λ_i x^(i−1) — in characteristic 2,
            // λ_i x^(i−1) for odd i.
            let mut den = 0u8;
            let mut i = 1;
            while i <= l {
                let mut t = lambda[i];
                for _ in 0..i - 1 {
                    t = GF.mul(t, xinv);
                }
                den ^= t;
                i += 2;
            }
            if den == 0 {
                return Err(Uncorrectable);
            }
            // X^(1 − fcr) = X · (X^fcr)⁻¹.
            let xf = GF.pow((PRIM * e * fcr) % 255);
            let y = GF.mul(GF.div(GF.mul(num, x), den), GF.div(1, xf));
            // Degree e is symbol n − 1 − e of the shortened word.
            fixed[self.n - 1 - e] ^= y;
        }
        for (c, f) in cw.iter_mut().zip(fixed) {
            *c = if dual { GF.to_dual[f as usize] } else { f };
        }
        Ok(locs.len())
    }
}
