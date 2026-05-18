use std::{collections::HashMap, io::Read, path::Path, str};

use adler32::adler32;
use anyhow::{Result, anyhow};
use memmap2::Mmap;

use encoding::{Encoding, all::UTF_16LE};
use flate2::read::ZlibDecoder;
use nom::{
    IResult, Slice,
    bytes::complete::{take, take_till},
    combinator::map,
    multi::{count, length_data, many0},
    number::complete::{be_u8, be_u16, be_u32, be_u64, le_u32},
    sequence::tuple,
};
use regex::Regex;
use ripemd::{Digest, Ripemd128};
use salsa20::{Salsa20, cipher::KeyIvInit};

use super::mdict::Mdx;

fn ne(e: nom::Err<nom::error::Error<&[u8]>>) -> anyhow::Error {
    anyhow!("{:?}", e)
}

// ── public zero-copy view (lifetime tied to KeyBlock::data) ──────────────────

#[derive(Debug)]
pub struct KeyEntry<'a> {
    pub offset: usize,
    pub text: &'a [u8],
}

// ── internal storage (no references, plain offsets) ──────────────────────────

#[derive(Debug)]
pub(crate) struct KeyEntrySlice {
    pub(crate) offset: usize,
    text_start: usize,
    text_len: usize,
}

#[derive(Debug)]
pub(crate) struct KeyBlock {
    pub(crate) data: Vec<u8>, // owns the decompressed block
    pub(crate) entries: Vec<KeyEntrySlice>,
}

impl KeyBlock {
    pub(crate) fn entries(&self) -> impl Iterator<Item = KeyEntry<'_>> {
        self.entries.iter().map(|e| KeyEntry {
            offset: e.offset,
            text: &self.data[e.text_start..e.text_start + e.text_len],
        })
    }

    pub(crate) fn first_key(&self) -> Option<&[u8]> {
        self.entries
            .first()
            .map(|e| &self.data[e.text_start..e.text_start + e.text_len])
    }

    pub(crate) fn last_key(&self) -> Option<&[u8]> {
        self.entries
            .last()
            .map(|e| &self.data[e.text_start..e.text_start + e.text_len])
    }

    pub(crate) fn lookup(&self, key: &str, encoding: &str) -> Vec<KeyEntry<'_>> {
        // MDict dictionaries use a collation where combining forms (e.g. "cat-") and
        // abbreviations (e.g. "cat.") sort before the bare headword ("cat"), which is
        // the opposite of Rust's standard string ordering. Binary search would therefore
        // miss entries whose neighbours differ only in a trailing punctuation character.
        // A linear scan over the block (typically a few hundred to a few thousand entries)
        // is both correct and fast enough for interactive use.
        //
        // Exact case match: the caller is responsible for passing the key in the desired
        // case. Block-level navigation uses lowercase for range checks; entry-level
        // matching here is case-sensitive so "cat" does not return "CAT".
        self.entries
            .iter()
            .filter(|e| {
                if let Ok(probe) = crate::lang::decode(
                    encoding,
                    &self.data[e.text_start..e.text_start + e.text_len],
                ) {
                    debug!("probe={}, key={}", probe, key);
                    if &probe == key {
                        return true;
                    }
                }

                false
            })
            .map(|entry| KeyEntry {
                offset: entry.offset,
                text: &self.data[entry.text_start..entry.text_start + entry.text_len],
            })
            .collect()
    }
}

// ── remaining parser types ────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Header {
    version: Version,
    encrypted: u8,
    encoding: String,
}

#[derive(Debug)]
struct KeyBlockHeader {
    block_num: usize,
    entry_num: usize,
    decompressed_size: usize,
    block_info_size: usize,
    key_block_size: usize,
}

#[derive(Debug)]
pub(crate) struct BlockEntryInfo {
    pub(crate) compressed_size: usize,
    pub(crate) decompressed_size: usize,
}

#[derive(Debug)]
enum Version {
    V1,
    V2,
    V3,
}

fn parse_header(input: &[u8]) -> Result<(&[u8], Header)> {
    let (input, (info, chksum)) = tuple((length_data(be_u32), le_u32))(input).map_err(ne)?;

    if adler32(info)? != chksum {
        bail!("header checksum mismatch");
    }

    let info = UTF_16LE
        .decode(info, encoding::DecoderTrap::Strict)
        .map_err(|e| anyhow!("{}", e))?;
    let attrs = parse_key_value(info.as_str());

    let version_str = attrs
        .get("GeneratedByEngineVersion")
        .ok_or_else(|| anyhow!("missing GeneratedByEngineVersion"))?;
    let version = version_str
        .trim()
        .slice(0..1)
        .parse::<u8>()
        .map_err(|e| anyhow!("invalid version: {}", e))?;

    let version = match version {
        1 => Version::V1,
        2 => Version::V2,
        3 => Version::V3,
        v => return Err(anyhow!("unsupported version: {}", v)),
    };

    let encrypted = attrs
        .get("Encrypted")
        .and_then(|x| match x == "Yes" {
            true => Some(1_u8),
            false => x.as_str().parse().ok(),
        })
        .unwrap_or(0);

    let encoding = attrs
        .get("Encoding")
        .unwrap_or(&"UTF-8".to_string())
        .to_string();

    Ok((
        input,
        Header {
            version,
            encrypted,
            encoding,
        },
    ))
}

fn parse_key_value(s: &str) -> HashMap<String, String> {
    let re = Regex::new(r#"(\w+)="((.|\r\n|[\r\n])*?)""#).expect("hardcoded regex is valid");
    let mut attrs = HashMap::new();
    for cap in re.captures_iter(s) {
        attrs.insert(cap[1].to_string(), cap[2].to_string());
    }
    attrs
}

fn parse_key_block_header_v2(input: &[u8]) -> Result<(&[u8], KeyBlockHeader)> {
    let (input, block_info_buf) = take(40_usize)(input).map_err(ne)?;
    let (input, chksum) = be_u32(input).map_err(ne)?;

    if adler32(block_info_buf)? != chksum {
        return Err(anyhow!("key block header checksum mismatch"));
    }

    let (_, res) = map(
        tuple((be_u64, be_u64, be_u64, be_u64, be_u64)),
        |(block_num, entry_num, decompressed_size, block_info_size, key_block_size)| {
            KeyBlockHeader {
                block_num: block_num as usize,
                entry_num: entry_num as usize,
                decompressed_size: decompressed_size as usize,
                block_info_size: block_info_size as usize,
                key_block_size: key_block_size as usize,
            }
        },
    )(block_info_buf)
    .map_err(ne)?;

    Ok((input, res))
}

fn parse_key_block_header_v1(input: &[u8]) -> Result<(&[u8], KeyBlockHeader)> {
    let (input, block_info_buf) = take(16_usize)(input).map_err(ne)?;

    let (_, res) = map(
        tuple((be_u32, be_u32, be_u32, be_u32)),
        |(block_num, entry_num, block_info_size, key_block_size)| KeyBlockHeader {
            block_num: block_num as usize,
            entry_num: entry_num as usize,
            decompressed_size: block_info_size as usize,
            block_info_size: block_info_size as usize,
            key_block_size: key_block_size as usize,
        },
    )(block_info_buf)
    .map_err(ne)?;

    Ok((input, res))
}

fn parse_key_block_header<'a>(
    input: &'a [u8],
    header: &Header,
) -> Result<(&'a [u8], KeyBlockHeader)> {
    match header.version {
        Version::V2 => parse_key_block_header_v2(input),
        Version::V1 => parse_key_block_header_v1(input),
        _ => Err(anyhow!("unsupported version")),
    }
}

fn parse_key_block_infos<'a>(
    input: &'a [u8],
    size: usize,
    dict_header: &Header,
) -> Result<(&'a [u8], Vec<BlockEntryInfo>)> {
    match dict_header.version {
        Version::V1 => parse_key_block_infos_v1(input, size),
        Version::V2 => parse_key_block_infos_v2(input, size, dict_header),
        _ => Err(anyhow!("unsupported version")),
    }
}

fn parse_key_block_infos_v1(input: &[u8], size: usize) -> Result<(&[u8], Vec<BlockEntryInfo>)> {
    let (input, block_info) = take(size)(input).map_err(ne)?;
    let entry_infos = decode_key_block_info_v1(block_info)?;
    Ok((input, entry_infos))
}

fn parse_key_block_infos_v2<'a>(
    input: &'a [u8],
    size: usize,
    dict_header: &Header,
) -> Result<(&'a [u8], Vec<BlockEntryInfo>)> {
    let (input, block_info) = take(size)(input).map_err(ne)?;

    if block_info.slice(0..4) != b"\x02\x00\x00\x00" {
        return Err(anyhow!("invalid key block info magic bytes"));
    }
    let mut key_block_info = vec![];

    if dict_header.encrypted == 2 || dict_header.encrypted == 3 {
        let mut md = Ripemd128::new();
        let mut v = Vec::from(block_info.slice(4..8));
        let value: u32 = 0x3695;
        v.extend_from_slice(&value.to_le_bytes());
        md.update(v);
        let key = md.finalize();
        let mut d = Vec::from(&block_info[0..8]);
        let decrypte = fast_decrypt(&block_info[8..], key.as_slice());
        d.extend(decrypte);
        ZlibDecoder::new(&d[8..]).read_to_end(&mut key_block_info)?;
    }
    if dict_header.encrypted == 0 {
        ZlibDecoder::new(&block_info[8..]).read_to_end(&mut key_block_info)?;
    }

    let entry_infos = decode_key_block_info_v2(&key_block_info)?;
    Ok((input, entry_infos))
}

fn text_len_parser_v2(input: &[u8]) -> IResult<&[u8], u16> {
    let (input, len) = be_u16(input)?;
    Ok((input, len + 1))
}

fn text_len_parser_v1(input: &[u8]) -> IResult<&[u8], u8> {
    be_u8(input)
}

fn decode_key_block_info_v1(input: &[u8]) -> Result<Vec<BlockEntryInfo>> {
    let mut info_parser = many0(map(
        tuple((
            be_u32,
            length_data(text_len_parser_v1),
            length_data(text_len_parser_v1),
            be_u32,
            be_u32,
        )),
        |(_, _, _, compressed_size, decompressed_size)| BlockEntryInfo {
            compressed_size: compressed_size as usize,
            decompressed_size: decompressed_size as usize,
        },
    ));
    let (remain, res) = info_parser(input).map_err(ne)?;
    if !remain.is_empty() {
        return Err(anyhow!("unexpected trailing bytes in key block info v1"));
    }
    Ok(res)
}

fn decode_key_block_info_v2(input: &[u8]) -> Result<Vec<BlockEntryInfo>> {
    let mut info_parser = many0(map(
        tuple((
            be_u64,
            length_data(text_len_parser_v2),
            length_data(text_len_parser_v2),
            be_u64,
            be_u64,
        )),
        |(_, _, _, compressed_size, decompressed_size)| BlockEntryInfo {
            compressed_size: compressed_size as usize,
            decompressed_size: decompressed_size as usize,
        },
    ));
    let (remain, res) = info_parser(input).map_err(ne)?;
    if !remain.is_empty() {
        return Err(anyhow!("unexpected trailing bytes in key block info v2"));
    }
    Ok(res)
}

fn parse_key_blocks<'a>(
    input: &'a [u8],
    size: usize,
    header: &Header,
    block_infos: &[BlockEntryInfo],
) -> Result<(&'a [u8], Vec<KeyBlock>)> {
    let (input, buf) = take(size)(input).map_err(ne)?;

    let blocks = match header.version {
        Version::V1 | Version::V2 => decode_blocks(buf, block_infos, header)?,
        _ => return Err(anyhow!("unsupported version")),
    };

    Ok((input, blocks))
}

fn decode_blocks(
    buf: &[u8],
    entry_infos: &[BlockEntryInfo],
    header: &Header,
) -> Result<Vec<KeyBlock>> {
    let mut buf = buf;
    let mut res = vec![];
    for info in entry_infos.iter() {
        let (remain, data) =
            block_parser(info.compressed_size, info.decompressed_size)(buf).map_err(ne)?;
        let entries = match header.version {
            Version::V1 => parse_block_items_v1(&data)?,
            Version::V2 => parse_block_items_v2(&data)?,
            _ => return Err(anyhow!("unsupported version")),
        };
        buf = remain;
        res.push(KeyBlock { data, entries });
    }
    Ok(res)
}

// Record text offsets relative to the start of the decompressed block buffer.
// KeyEntry<'a> reconstructs &'a [u8] slices from these at access time.
fn parse_block_items_v1(input: &[u8]) -> Result<Vec<KeyEntrySlice>> {
    let base = input.as_ptr() as usize;
    let (remain, sep) = many0(map(
        tuple((be_u32, take_till(|x| x == 0), take(1_usize))),
        |(offset, buf, _): (u32, &[u8], &[u8])| KeyEntrySlice {
            offset: offset as usize,
            text_start: buf.as_ptr() as usize - base,
            text_len: buf.len(),
        },
    ))(input)
    .map_err(ne)?;

    if !remain.is_empty() {
        return Err(anyhow!("unexpected trailing bytes in key block v1"));
    }
    Ok(sep)
}

fn parse_block_items_v2(input: &[u8]) -> Result<Vec<KeyEntrySlice>> {
    let base = input.as_ptr() as usize;
    let (remain, sep) = many0(map(
        tuple((be_u64, take_till(|x| x == 0), take(1_usize))),
        |(offset, buf, _): (u64, &[u8], &[u8])| KeyEntrySlice {
            offset: offset as usize,
            text_start: buf.as_ptr() as usize - base,
            text_len: buf.len(),
        },
    ))(input)
    .map_err(ne)?;

    if !remain.is_empty() {
        return Err(anyhow!("unexpected trailing bytes in key block v2"));
    }
    Ok(sep)
}

fn block_parser<'a>(
    comp_size: usize,
    decomp_size: usize,
) -> impl FnMut(&'a [u8]) -> IResult<&'a [u8], Vec<u8>> {
    map(
        tuple((le_u32, take(4_usize), take(comp_size - 8))),
        move |(enc, chksum, encrypted)| {
            let enc_method = (enc >> 4) & 0xf;
            let _enc_size = (enc >> 8) & 0xff;
            let comp_method = enc & 0xf;

            let mut md = Ripemd128::new();
            md.update(chksum);
            let key = md.finalize();

            let data: Vec<u8> = match enc_method {
                0 => Vec::from(encrypted),
                1 => fast_decrypt(encrypted, key.as_slice()),
                2 => {
                    let decrypt = vec![];
                    let _cipher = Salsa20::new(key.as_slice().into(), &[0; 8].into());
                    decrypt
                }
                _ => panic!("unknown enc method: {}", enc_method),
            };

            match comp_method {
                0 => data,
                1 => {
                    let mut comp: Vec<u8> = vec![0xf0];
                    comp.extend_from_slice(&data[..]);
                    let lzo = minilzo_rs::LZO::init().expect("LZO init failed");
                    lzo.decompress(&data[..], decomp_size)
                        .expect("LZO decompression failed")
                }
                2 => {
                    let mut v = vec![];
                    ZlibDecoder::new(&data[..])
                        .read_to_end(&mut v)
                        .expect("zlib decompression failed");
                    v
                }
                _ => panic!("unknown compression method: {}", comp_method),
            }
        },
    )
}

fn parse_record_blocks<'a>(
    input: &'a [u8],
    header: &Header,
) -> Result<(&'a [u8], Vec<BlockEntryInfo>)> {
    match header.version {
        Version::V1 => parse_record_blocks_v1(input),
        Version::V2 => parse_record_blocks_v2(input),
        _ => Err(anyhow!("unsupported version")),
    }
}

fn parse_record_blocks_v1(input: &[u8]) -> Result<(&[u8], Vec<BlockEntryInfo>)> {
    let (input, records) = be_u32(input).map_err(ne)?;
    let (input, _entries) = be_u32(input).map_err(ne)?;
    let (input, record_info_size) = be_u32(input).map_err(ne)?;
    let (input, _record_buf_size) = be_u32(input).map_err(ne)?;

    if records * 8 != record_info_size {
        return Err(anyhow!("record info size mismatch (v1)"));
    }

    let (input, res) = count(
        map(
            tuple((be_u32, be_u32)),
            |(compressed_size, decompressed_size)| BlockEntryInfo {
                compressed_size: compressed_size as usize,
                decompressed_size: decompressed_size as usize,
            },
        ),
        records as usize,
    )(input)
    .map_err(ne)?;

    Ok((input, res))
}

fn parse_record_blocks_v2(input: &[u8]) -> Result<(&[u8], Vec<BlockEntryInfo>)> {
    let (input, records) = be_u64(input).map_err(ne)?;
    let (input, _entries) = be_u64(input).map_err(ne)?;
    let (input, record_info_size) = be_u64(input).map_err(ne)?;
    let (input, _record_buf_size) = be_u64(input).map_err(ne)?;

    if records * 16 != record_info_size {
        return Err(anyhow!("record info size mismatch (v2)"));
    }

    let (input, res) = count(
        map(
            tuple((be_u64, be_u64)),
            |(compressed_size, decompressed_size)| BlockEntryInfo {
                compressed_size: compressed_size as usize,
                decompressed_size: decompressed_size as usize,
            },
        ),
        records as usize,
    )(input)
    .map_err(ne)?;

    Ok((input, res))
}

fn fast_decrypt(encrypted: &[u8], key: &[u8]) -> Vec<u8> {
    let mut buf = Vec::from(encrypted);
    let mut prev = 0x36;
    for i in 0..buf.len() {
        let mut t = buf[i] >> 4 | buf[i] << 4;
        t = t ^ prev ^ (i as u8) ^ key[i % key.len()];
        prev = buf[i];
        buf[i] = t;
    }
    buf
}

pub(crate) fn record_block_parser<'a>(
    size: usize,
    decomp_size: usize,
) -> impl FnMut(&'a [u8]) -> IResult<&'a [u8], Vec<u8>> {
    map(
        tuple((le_u32, take(4_usize), take(size - 8))),
        move |(enc, chksum, encrypted)| {
            let enc_method = (enc >> 4) & 0xf;
            let _enc_size = (enc >> 8) & 0xff;
            let comp_method = enc & 0xf;

            let mut md = Ripemd128::new();
            md.update(chksum);
            let key = md.finalize();

            let data: Vec<u8> = match enc_method {
                0 => Vec::from(encrypted),
                1 => fast_decrypt(encrypted, key.as_slice()),
                2 => {
                    let decrypt = vec![];
                    let _cipher = Salsa20::new(key.as_slice().into(), &[0; 8].into());
                    decrypt
                }
                _ => panic!("unknown enc method: {}", enc_method),
            };

            match comp_method {
                0 => data,
                1 => {
                    let lzo = minilzo_rs::LZO::init().expect("LZO init failed");
                    lzo.decompress(&data[..], decomp_size)
                        .expect("LZO decompression failed")
                }
                2 => {
                    let mut v = vec![];
                    ZlibDecoder::new(&data[..])
                        .read_to_end(&mut v)
                        .expect("zlib decompression failed");
                    v
                }
                _ => panic!("unknown compression method: {}", comp_method),
            }
        },
    )
}

pub fn parse<P: AsRef<Path>>(path: P) -> Result<Mdx> {
    let file = std::fs::File::open(path.as_ref())?;
    let mmap = unsafe { Mmap::map(&file)? };

    let (input, header) = parse_header(&mmap)?;
    let (input, key_block_header) = parse_key_block_header(input, &header)?;
    let (input, key_block_infos) =
        parse_key_block_infos(input, key_block_header.block_info_size, &header)?;
    let (input, key_blocks) = parse_key_blocks(
        input,
        key_block_header.key_block_size,
        &header,
        &key_block_infos,
    )?;
    let (input, record_blocks) = parse_record_blocks(input, &header)?;

    let records_start = mmap.len() - input.len();

    Ok(Mdx {
        key_blocks,
        records_info: record_blocks,
        mmap,
        records_start,
        encoding: header.encoding,
        encrypted: header.encrypted,
    })
}
