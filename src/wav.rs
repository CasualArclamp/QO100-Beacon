// made by CasualArclamp
//! Reads a WAV recording in blocks: one channel as a real signal (a WebSDR's
//! or a receiver's audio), two as IQ (I left, Q right). 8-, 16- and 24-bit
//! PCM and 32-bit float. After DecSAT's reader (`decsat-io/src/wav.rs`).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::Iq;

pub struct WavReader {
    inner: BufReader<File>,
    sample_rate: f64,
    channels: usize,
    /// Bytes per sample of one channel, and whether they are floats.
    width: usize,
    float: bool,
    /// Bytes of the data chunk still to read.
    left: u64,
    raw: Vec<u8>,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("WAV: {msg}"))
}

impl WavReader {
    /// Open `path`, to be read `block` samples at a time.
    pub fn open(path: &Path, block: usize) -> io::Result<Self> {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        let mut head = [0u8; 12];
        f.read_exact(&mut head)?;
        if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
            return Err(invalid("not a RIFF WAVE file"));
        }
        let mut fmt: Option<(u16, u16, u32, u16)> = None;
        let mut at = 12u64;
        // Walk the chunks: id, 32-bit size, body (padded to an even length).
        while at + 8 <= len {
            f.seek(SeekFrom::Start(at))?;
            let mut ch = [0u8; 8];
            f.read_exact(&mut ch)?;
            let size = u64::from(u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]));
            let body = at + 8;
            match &ch[0..4] {
                b"fmt " => {
                    let mut b = vec![0u8; size.min(64) as usize];
                    f.read_exact(&mut b)?;
                    if b.len() < 16 {
                        return Err(invalid("short fmt chunk"));
                    }
                    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
                    let mut tag = u16_at(0);
                    // WAVE_FORMAT_EXTENSIBLE: the real tag opens the
                    // sub-format GUID.
                    if tag == 0xFFFE && b.len() >= 26 {
                        tag = u16_at(24);
                    }
                    let rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                    fmt = Some((tag, u16_at(2), rate, u16_at(14)));
                }
                b"data" => {
                    let (tag, channels, rate, bits) =
                        fmt.ok_or_else(|| invalid("no fmt chunk before the data"))?;
                    if channels != 1 && channels != 2 {
                        return Err(invalid("one channel (audio) or two (IQ) only"));
                    }
                    let (width, float) = match (tag, bits) {
                        (1, 8) => (1, false),
                        (1, 16) => (2, false),
                        (1, 24) => (3, false),
                        (3, 32) => (4, true),
                        _ => return Err(invalid("8/16/24-bit PCM or 32-bit float only")),
                    };
                    f.seek(SeekFrom::Start(body))?;
                    let channels = usize::from(channels);
                    return Ok(WavReader {
                        inner: BufReader::with_capacity(1 << 16, f),
                        sample_rate: f64::from(rate),
                        channels,
                        width,
                        float,
                        // A data chunk cut short by an interrupted
                        // recording: read what is there.
                        left: size.min(len.saturating_sub(body)),
                        raw: vec![0u8; block.max(1) * channels * width],
                    });
                }
                _ => {}
            }
            at = body + size + (size & 1);
        }
        Err(invalid("no data chunk"))
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    fn value(&self, b: &[u8]) -> f32 {
        match (self.width, self.float) {
            // 8-bit WAV samples are unsigned.
            (1, _) => (f32::from(b[0]) - 128.0) / 128.0,
            (2, _) => f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
            (3, _) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
            _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        }
    }

    /// Read the next block into `out` (cleared first); 0 at the end.
    pub fn read(&mut self, out: &mut Vec<Iq>) -> io::Result<usize> {
        out.clear();
        let want = (self.left.min(self.raw.len() as u64)) as usize;
        let mut filled = 0;
        while filled < want {
            match self.inner.read(&mut self.raw[filled..want])? {
                0 => break,
                n => filled += n,
            }
        }
        self.left -= filled as u64;
        let frame = self.channels * self.width;
        for f in self.raw[..filled - filled % frame].chunks(frame) {
            let i = self.value(f);
            let q = if self.channels == 2 {
                self.value(&f[self.width..])
            } else {
                0.0
            };
            out.push(Iq::new(i, q));
        }
        Ok(out.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, bits: u16, tag: u16, rate: u32, data: &[u8]) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&tag.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&rate.to_le_bytes());
        body.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        body.extend_from_slice(&block.to_le_bytes());
        body.extend_from_slice(&bits.to_le_bytes());
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        // A trailing chunk that must not be read as samples.
        body.extend_from_slice(b"LIST\x04\x00\x00\x00INFO");
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn read_all(name: &str, bytes: &[u8]) -> (f64, Vec<Iq>) {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        let mut r = WavReader::open(&path, 2).unwrap();
        let mut all = Vec::new();
        let mut block = Vec::new();
        while r.read(&mut block).unwrap() > 0 {
            all.extend_from_slice(&block);
        }
        let _ = std::fs::remove_file(path);
        (r.sample_rate(), all)
    }

    #[test]
    fn mono_sixteen_bit_is_a_real_signal() {
        let data: Vec<u8> = [16384i16, -16384, 8192]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let (rate, got) = read_all("qo100-mono.wav", &wav(1, 16, 1, 8000, &data));
        assert_eq!(rate, 8000.0);
        assert_eq!(
            got,
            vec![Iq::new(0.5, 0.0), Iq::new(-0.5, 0.0), Iq::new(0.25, 0.0)]
        );
    }

    #[test]
    fn stereo_is_iq_and_other_widths_read() {
        let data: Vec<u8> = [16384i16, -16384]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let (_, got) = read_all("qo100-iq.wav", &wav(2, 16, 1, 48_000, &data));
        assert_eq!(got, vec![Iq::new(0.5, -0.5)]);
        let (_, got) = read_all("qo100-8.wav", &wav(1, 8, 1, 8000, &[192, 64]));
        assert_eq!(got, vec![Iq::new(0.5, 0.0), Iq::new(-0.5, 0.0)]);
        let (_, got) = read_all(
            "qo100-24.wav",
            &wav(1, 24, 1, 8000, &[0, 0, 0x40, 0, 0, 0xC0]),
        );
        assert_eq!(got, vec![Iq::new(0.5, 0.0), Iq::new(-0.5, 0.0)]);
        let float: Vec<u8> = 0.25f32.to_le_bytes().to_vec();
        let (_, got) = read_all("qo100-f.wav", &wav(1, 32, 3, 8000, &float));
        assert_eq!(got, vec![Iq::new(0.25, 0.0)]);
    }

    #[test]
    fn other_files_are_refused() {
        let path = std::env::temp_dir().join("qo100-not.wav");
        std::fs::write(&path, b"not a wav file at all").unwrap();
        assert!(WavReader::open(&path, 2).is_err());
        let _ = std::fs::remove_file(path);
    }
}
