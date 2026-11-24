/// ITU-T G.711 A-law and Mu-law codec
pub struct G711;

const SEG_AEND: [i32; 8] = [0x1F, 0x3F, 0x7F, 0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF];

impl G711 {
    /// Encodes a 16-bit linear PCM sample into an 8-bit A-law byte (ITU-T G.711)
    pub fn linear_to_alaw(mut pcm_val: i16) -> u8 {
        let mask = if pcm_val >= 0 {
            0xD5 // sign bit = 1
        } else {
            pcm_val = -pcm_val - 1;
            0x55 // sign bit = 0
        };

        let mut val = (pcm_val as i32) >> 3;
        if val < 0 {
            val = 0;
        }

        let mut seg = 8;
        for i in 0..8 {
            if val <= SEG_AEND[i] {
                seg = i;
                break;
            }
        }

        let aval = if seg >= 8 {
            0x7F ^ mask
        } else {
            let mut a = (seg as u8) << 4;
            if seg < 2 {
                a |= ((val >> 1) & 0x0F) as u8;
            } else {
                a |= ((val >> seg) & 0x0F) as u8;
            }
            a ^ mask
        };

        aval
    }

    /// Decodes an 8-bit A-law byte into a 16-bit linear PCM sample
    pub fn alaw_to_linear(mut a_val: u8) -> i16 {
        a_val ^= 0x55;

        let mut t = ((a_val & 0x0F) as i32) << 4;
        let seg = ((a_val & 0x70) >> 4) as usize;

        match seg {
            0 => t += 8,
            1 => t += 0x108,
            _ => {
                t += 0x108;
                t <<= seg - 1;
            }
        }

        if (a_val & 0x80) != 0 {
            t as i16
        } else {
            -t as i16
        }
    }

    /// Encodes a 16-bit linear PCM sample into an 8-bit Mu-law byte (ITU-T G.711)
    pub fn linear_to_mulaw(mut pcm_val: i16) -> u8 {
        let sign = if pcm_val < 0 {
            pcm_val = -pcm_val;
            0x80u8
        } else {
            0x00u8
        };

        if pcm_val > 8159 {
            pcm_val = 8159;
        }
        pcm_val += 0x84;

        let mut seg = 7;
        let mut mask_seg = 0x4000;
        while (pcm_val as i32 & mask_seg) == 0 && seg > 0 {
            seg -= 1;
            mask_seg >>= 1;
        }

        let mantissa = ((pcm_val >> (seg + 3)) & 0x0F) as u8;
        let u_val = sign | ((seg as u8) << 4) | mantissa;
        !u_val
    }

    /// Decodes an 8-bit Mu-law byte into a 16-bit linear PCM sample (ITU-T G.711)
    pub fn mulaw_to_linear(u_val: u8) -> i16 {
        let u_val = !u_val;
        let sign = (u_val & 0x80) != 0;
        let exponent = ((u_val >> 4) & 0x07) as usize;
        let mantissa = (u_val & 0x0F) as i32;
        let sample = (((mantissa << 3) + 0x84) << exponent) - 0x84;
        if sign {
            -(sample as i16)
        } else {
            sample as i16
        }
    }

    /// Generates 20ms of silence in A-law (160 samples at 8000Hz)
    pub fn generate_silence_alaw(samples: usize) -> Vec<u8> {
        vec![0xD5; samples]
    }

    /// Generates 20ms of a sine tone (e.g. 440Hz dial tone) in A-law
    pub fn generate_tone_alaw(freq_hz: f32, sample_offset: usize, count: usize) -> Vec<u8> {
        let mut buf = Vec::with_capacity(count);
        for i in 0..count {
            let t = (sample_offset + i) as f32 / 8000.0;
            let sample = (t * freq_hz * 2.0 * std::f32::consts::PI).sin();
            let pcm = (sample * 16384.0) as i16;
            buf.push(Self::linear_to_alaw(pcm));
        }
        buf
    }
}
