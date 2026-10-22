//! 微软 Edge 朗读（edge-tts）的 DRM token 算法。
//!
//! Edge 的在线朗读接口没有 API key，用 `Sec-MS-GEC` 查询参数做反滥用校验：
//! token 由「自 Windows FILETIME 纪元起、按 5 分钟取整的时间戳 + 固定客户端
//! token」派生。旧版 v1（简化 token）已被服务端 403 拒绝，必须用 v2：
//! `<16位CS64 hex>-<64位SHA-256变体 hex>`。
//!
//! 本文件是对参考实现 `edge_tts/drm.py`（edge-tts 7.3.1）的逐位移植。它并不是
//! 标准 SHA-256/CS64 —— 初始状态、第 14 轮常数与 CS64 系数都被改过，所以不能
//! 换成 `sha2` crate 实现；下面的常量与算法必须与那份代码保持一致，任何「顺手
//! 优化」都可能导致服务端 403。单测里的向量由官方 Python 实现生成，改实现前先
//! 跑向量。

/// Windows FILETIME 纪元与 Unix 纪元的秒差（1601-01-01 → 1970-01-01）。
const WIN_EPOCH: f64 = 11_644_473_600.0;
/// 固定客户端 token，所有 Edge 朗读请求共用。
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// 时间戳按 5 分钟取整（服务端同一窗口内接受同一 token）。
const TOKEN_WINDOW_SECONDS: f64 = 300.0;

const SHA256_INITIAL: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB,
    0x5BE0CD19,
];

/// 移植版的「初始压缩状态」：它不是标准 IV，是参考实现里对前 7 字节的预计算状态。
const SHA256_STATE: [u32; 8] = [
    0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4,
    0x03EFFBD2,
];

const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0,
    0xF9E70AB6, 0xF0C31573, 0x7058CE4E, 0x7188E606, 0x00E5AB33, 0xD258B060, 0x72766564,
];

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4,
    0xAB1C5ED5, 0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE,
    0x9BDC06A7, 0xC19BF174, 0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F,
    0x4A7484AA, 0x5CB0A9DC, 0x76F988DA, 0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7,
    0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967, 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC,
    0x53380D13, 0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85, 0xA2BFE8A1, 0xA81A664B,
    0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070, 0x19A4C116,
    0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3,
    0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7,
    0xC67178F2,
];

const SHA256_ADJUSTMENT: u32 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;

/// CS64 的 14 组（系数, 偏移）常量。
const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

/// 生成当前时刻的 `Sec-MS-GEC` token（连接/列表请求时现算）。
pub fn generate_sec_ms_gec() -> String {
    generate_sec_ms_gec_at(current_unix_timestamp())
}

/// 用指定 Unix 时间戳生成 token。时间戳参数只是为了单测可复现。
pub fn generate_sec_ms_gec_at(unix_timestamp: f64) -> String {
    // 与服务端保持 5 分钟对齐：同一时间窗内 token 稳定，跨窗后自动变化。
    let mut ticks = unix_timestamp + WIN_EPOCH;
    ticks -= ticks % TOKEN_WINDOW_SECONDS;
    // 参考实现里是 `ticks *= 1e9 / 100`，即乘以 1e7；必须保持 f64 运算顺序与
    // 四舍五入方式，因为 1.3e17 量级的乘积累加会丢失低位，token 依赖这些位。
    ticks *= 1e9 / 100.0;
    let data = format!("{ticks:.0}{TRUSTED_CLIENT_TOKEN}");
    let (cs64, sha256) = token_hash(data.as_bytes());
    format!("{cs64}-{sha256}")
}

fn current_unix_timestamp() -> f64 {
    chrono::Utc::now().timestamp_micros() as f64 / 1_000_000.0
}

fn rol16(value: u32) -> u32 {
    value.rotate_left(16)
}

/// `_token_hash`：把 CS64 与 SHA-256 变体摘要拼成 token 的两段。
fn token_hash(data: &[u8]) -> (String, String) {
    let digest = sha256_digest(data);
    let (low, high) = cs64_digest(data, &digest);
    // 参考实现是 `struct.pack("<II", a, b).hex().upper()`：小端字节序展开。
    let mut cs64_bytes = [0u8; 8];
    cs64_bytes[..4].copy_from_slice(&low.to_le_bytes());
    cs64_bytes[4..].copy_from_slice(&high.to_le_bytes());
    (
        hex::encode_upper(cs64_bytes),
        hex::encode_upper(digest),
    )
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    assert!(data.len() >= 7, "token 数据至少 7 字节");
    // 前 7 字节交给定制的 `sha256_init`（含一个预计算块），剩余字节按标准
    // SHA-256 补齐后逐块压缩。这里的 length 用的是「57 + 总长」，与标准补齐
    // 等价但改成参考实现的写法，便于逐行对照。
    let length = 57 + data.len();
    let mut padded: Vec<u8> = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(&data[7..]);
    padded.push(0x80);
    let zeros = (55i64 - length as i64).rem_euclid(64) as usize;
    padded.extend(std::iter::repeat(0).take(zeros));
    padded.extend_from_slice(&((length as u64).wrapping_mul(8)).to_be_bytes());

    let mut state = sha256_init(&data[..7]);
    for block in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        expand_sha256_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for index in 0..8 {
            state[index] = state[index].wrapping_add(working[index]);
        }
    }

    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// 参考实现的「预计算初始状态」：吃掉 token 数据的前 7 字节。
fn sha256_init(data_prefix: &[u8]) -> [u32; 8] {
    let mut words = [0u32; 64];
    words[14] = ((data_prefix[0] as u32) << 16)
        | ((data_prefix[1] as u32) << 8)
        | data_prefix[2] as u32;
    words[15] = u32::from_be_bytes([
        data_prefix[3],
        data_prefix[4],
        data_prefix[5],
        data_prefix[6],
    ]);

    for index in 16..30usize {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index >= 16 {
            // index 16 的 sigma1 结果要异或一个调整常量；其余轮次用标准 sigma。
            let sigma1 = if index == 16 {
                sigma1(words[index - 2]) ^ SHA256_SIGMA1_ADJUSTMENT
            } else {
                sigma1(words[index - 2])
            };
            value = value.wrapping_add(sigma1);
        }
        if index >= 21 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index >= 29 {
            let sigma0 = if index == 29 {
                sigma0(words[index - 15]) ^ SHA256_SIGMA0_ADJUSTMENT
            } else {
                sigma0(words[index - 15])
            };
            value = value.wrapping_add(sigma0);
        }
        words[index] = value;
    }
    words[30] = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);

    expand_sha256_schedule(&mut words, 31);
    // 从第 14 轮开始跑（前 14 轮由这份预计算状态隐含），且第 14 轮常数被替换。
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    let mut state = [0u32; 8];
    for index in 0..8 {
        state[index] = SHA256_INITIAL[index].wrapping_add(working[index]);
    }
    state
}

/// 参考实现的小 sigma0：三个旋转合并后只截断一次。
fn sigma0(value: u32) -> u32 {
    (value >> 7) ^ value.wrapping_shl(25) ^ (value >> 18) ^ value.wrapping_shl(14) ^ (value >> 3)
}

/// 参考实现的小 sigma1：同上。
fn sigma1(value: u32) -> u32 {
    (value >> 17)
        ^ value.wrapping_shl(15)
        ^ (value >> 19)
        ^ value.wrapping_shl(13)
        ^ (value >> 10)
}

/// 展开 SHA-256 消息计划，从 `first_word` 开始到最后。
fn expand_sha256_schedule(words: &mut [u32; 64], first_word: usize) {
    for index in first_word..64 {
        let x = words[index - 2];
        let y = words[index - 15];
        words[index] = sigma1(x)
            .wrapping_add(words[index - 7])
            .wrapping_add(sigma0(y))
            .wrapping_add(words[index - 16]);
    }
}

/// 跑 SHA-256 压缩轮。`first_round == 14` 时使用被改过的第 14 轮常数。
fn sha256_rounds(state: [u32; 8], words: &[u32; 64], first_round: usize) -> [u32; 8] {
    let constants = if first_round == 14 {
        &BLOCK_ROUND_CONSTANTS
    } else {
        &SHA256_ROUND_CONSTANTS
    };
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for index in first_round..64 {
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = g ^ (e & (f ^ g));
        let t1 = h
            .wrapping_add(sum1)
            .wrapping_add(choose)
            .wrapping_add(constants[index])
            .wrapping_add(words[index]);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) | (c & (a | b));
        let t2 = sum0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    [a, b, c, d, e, f, g, h]
}

/// 参考实现里第 14 轮常数被换成了 `SHA256_ADJUSTMENT`。
const BLOCK_ROUND_CONSTANTS: [u32; 64] = {
    let mut constants = SHA256_ROUND_CONSTANTS;
    constants[14] = SHA256_ADJUSTMENT;
    constants
};

fn words_le4_even(input: &[u8]) -> Vec<u32> {
    let mut padded = input.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    let mut words: Vec<u32> = padded
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    // 参考实现把奇数个 32 位字补到偶数个（后面的按对循环依赖这一点）。
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

/// CS64 摘要：返回（异或后的 state, 异或后的 acc），供 `token_hash` 拼 token。
fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let factors = [
        key_a.wrapping_mul(CS64_FACTOR),
        key_b.wrapping_mul(CS64_FACTOR),
    ];

    let mut terms: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .enumerate()
        .map(|(index, (coefficient, _))| coefficient.wrapping_mul(factors[index % 2]))
        .collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS
        .iter()
        .map(|(_, offset)| *offset)
        .collect();

    // 数据前置一个 0 字节，按小端 32 位字喂进 CS64。
    let mut prefixed = Vec::with_capacity(data.len() + 1);
    prefixed.push(0u8);
    prefixed.extend_from_slice(data);
    for (index, word) in words_le4_even(&prefixed).into_iter().enumerate() {
        let key = if index % 2 == 0 { key_a } else { key_b };
        let mut term = word.wrapping_mul(key);
        let mut offset = rol16(word).wrapping_mul(if index % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }

    let mut swap_acc = 0u32;
    let mut swap_last = 0u32;
    let mut state = 0u32;
    let mut acc = 0u32;
    for index in (0..terms.len()).step_by(2) {
        let x = terms[index];
        let y = terms[index + 1];

        let t = rol16(offsets[index].wrapping_add(x));
        let q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));
        let t = rol16(offsets[index + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);

        let mut t = state.wrapping_mul(key_b).wrapping_add(y);
        t = rol16(t).wrapping_mul(0xA27416F5);
        t = rol16(t).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
    }

    (swap_last ^ state, swap_acc ^ acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 由官方 Python 实现（edge-tts 7.3.1 的 `generate_sec_ms_gec_v2`）生成的
    /// 向量。逐字符相等是 DRM 移植的唯一验收标准：差一个字符服务端就 403。
    #[test]
    fn drm_v2_tokens_match_official_vectors() {
        let vectors: [(f64, &str); 7] = [
            (
                0.0,
                "52BC7A17B50DFEC9-28C4394775C8751CD2F6859C030C06DB1489ACAB3B3A7651831FC26895177ED6",
            ),
            (
                1_700_000_000.0,
                "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2",
            ),
            (
                1_750_000_000.5,
                "B029A7426816F097-4579D112DC47FC3BF432B499C0A282F427C659474AF9BF5FA33CE2C003F9E9C2",
            ),
            (
                1_760_000_000.0,
                "6D50DDDB91812C33-DD68EFB01898B0750C9797D51A5FF93471DFDAE6888A1039D532CBDCCE54545D",
            ),
            (
                1_800_000_000.123,
                "99E54519B490BE5C-E162DB0977FBDCB9B86DD8148BF1BCDC3C0303176CE39852A42B415935C20654",
            ),
            // 额外两组同样由官方 Python 生成，覆盖 2025 / 2030 的 5 分钟窗口。
            (
                1_735_689_600.0,
                "54404AFE67C9B44A-E72E51D8B4F5F75E9A96B7E20D7877227455BC310433BD30C817CD54A1D9313E",
            ),
            (
                1_893_456_000.0,
                "7442B6E630CDF0DE-CE82EC1509335DC856270C1EEB0F7F2575909AC26840D2C15B28A41EBB7E9CCF",
            ),
        ];
        for (timestamp, expected) in vectors {
            assert_eq!(
                generate_sec_ms_gec_at(timestamp),
                expected,
                "timestamp {timestamp}"
            );
        }
    }

    #[test]
    fn token_shape_is_cs64_dash_sha256_uppercase() {
        let token = generate_sec_ms_gec_at(1_760_000_000.0);
        let (cs64, sha256) = token.split_once('-').expect("token 必须含 '-'");
        assert_eq!(cs64.len(), 16);
        assert_eq!(sha256.len(), 64);
        assert!(
            token
                .chars()
                .all(|ch| ch == '-' || (ch.is_ascii_hexdigit() && !ch.is_ascii_lowercase())),
            "token 必须全是大写 hex: {token}"
        );
    }

    /// 同一个 5 分钟窗口里的不同时间戳必须得到同一个 token（服务端按窗口校验）。
    #[test]
    fn token_is_stable_inside_five_minute_window() {
        // 1759999800 + WIN_EPOCH 正好落在 300 秒窗口的起点上，+100 仍在窗口内。
        let window_start = 1_759_999_800.0;
        for offset in [0.0, 1.0, 99.5, 299.0] {
            assert_eq!(
                generate_sec_ms_gec_at(window_start + offset),
                generate_sec_ms_gec_at(window_start),
                "offset {offset} 仍应在同一窗口"
            );
        }
        assert_ne!(
            generate_sec_ms_gec_at(window_start),
            generate_sec_ms_gec_at(window_start + 300.0)
        );
    }
}
