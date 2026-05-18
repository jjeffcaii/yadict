use super::parser::{BlockEntryInfo, KeyBlock, KeyEntry, record_block_parser};
use crate::lang::compare as icu_compare;
use encoding::{Encoding, all::UTF_16LE, label::encoding_from_whatwg_label};
use memmap2::Mmap;
use nom::{IResult, bytes::complete::take_till};
use std::cell::OnceCell;
use std::cmp::Ordering;
use std::fmt::Display;

#[derive(Debug)]
struct RecordOffset {
    buf_offset: usize,
    block_offset: usize,
    record_size: usize,
    decomp_size: usize,
}

pub struct Record<'a> {
    key: &'a [u8],
    mdx: &'a Mdx,
    entry_offset: usize,
    key_cache: OnceCell<String>,
    cache: OnceCell<String>,
}

impl<'a> Record<'a> {
    pub fn key(&self) -> &str {
        self.key_cache
            .get_or_init(|| {
                crate::lang::decode(&self.mdx.encoding, self.key).expect("failed to decode key")
            })
            .as_ref()
    }

    /// Lazily decompresses and returns the raw definition bytes.
    /// Returns `None` when no record block covers this entry's offset.
    /// The result is cached; repeated calls are free.
    pub fn value(&self) -> &str {
        self.cache
            .get_or_init(|| match self.mdx.fetch_definition(self.entry_offset) {
                Some(b) => {
                    crate::lang::decode(&self.mdx.encoding, &b).expect("failed to decode value")
                }
                None => String::new(),
            })
            .as_ref()
    }
}

pub struct Mdx {
    pub(crate) key_blocks: Vec<KeyBlock>,
    pub(crate) records_info: Vec<BlockEntryInfo>,
    pub(crate) mmap: Mmap,
    pub(crate) records_start: usize,
    pub encoding: String,
    pub encrypted: u8,
}

impl Mdx {
    pub fn items(&self) -> impl Iterator<Item = Record<'_>> + '_ {
        self.key_blocks.iter().flat_map(|block| {
            block.entries().map(|entry| Record {
                key: entry.text,
                mdx: self,
                entry_offset: entry.offset,
                key_cache: OnceCell::new(),
                cache: OnceCell::new(),
            })
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = KeyEntry<'_>> + '_ {
        self.key_blocks.iter().flat_map(|block| block.entries())
    }

    fn record_offset(&self, entry_offset: usize) -> Option<RecordOffset> {
        let mut block_offset = 0;
        let mut buf_offset = 0;
        for i in &self.records_info {
            if entry_offset < block_offset + i.decompressed_size {
                return Some(RecordOffset {
                    buf_offset,
                    block_offset: entry_offset - block_offset,
                    record_size: i.compressed_size,
                    decomp_size: i.decompressed_size,
                });
            } else {
                block_offset += i.decompressed_size;
                buf_offset += i.compressed_size;
            }
        }
        None
    }

    pub fn lookup<A>(&self, key: A) -> Vec<Record<'_>>
    where
        A: AsRef<str>,
    {
        let key = key.as_ref();

        // MDict dictionaries sort punctuated forms (e.g. "cat-", "cat.") before the bare
        // headword ("cat"). When such a form is a block's first_key, standard string
        // comparison ("cat-" > "cat") causes binary_search_by to skip that block entirely.
        //
        // Instead, use partition_point to find the insertion index, then probe both the
        // block just before the insertion point (normal case) and the block AT the insertion
        // point (edge case: its first_key is a punctuated variant of our key). The entry-level
        // search is a linear scan, so checking one extra block is cheap.
        let pos = self
            .key_blocks
            .partition_point(|probe| match probe.last_key() {
                None => false,
                Some(b) => {
                    if let Ok(end) = crate::lang::decode(key, &b) {
                        let ordering = icu_compare(&end, key);
                        debug!("probe={}, key={}, result={:?}", &end, key, ordering);
                        if Ordering::Less == ordering {
                            return true;
                        }
                    }
                    false
                }
            });

        debug!("pos: {}", pos);

        let prev_pos = pos.wrapping_sub(1);

        for idx in [prev_pos, pos] {
            if let Some(block) = self.key_blocks.get(idx) {
                let found = block
                    .lookup(key, &self.encoding)
                    .iter()
                    .map(|entry| Record {
                        key: entry.text,
                        mdx: self,
                        entry_offset: entry.offset,
                        key_cache: OnceCell::new(),
                        cache: OnceCell::new(),
                    })
                    .collect::<Vec<Record>>();

                if !found.is_empty() {
                    return found;
                }
            }
        }

        for idx in 0..self.key_blocks.len() {
            if idx == prev_pos || idx == pos {
                continue;
            }

            if let Some(block) = self.key_blocks.get(idx) {
                let found = block
                    .lookup(key, &self.encoding)
                    .iter()
                    .map(|entry| Record {
                        key: entry.text,
                        mdx: self,
                        entry_offset: entry.offset,
                        key_cache: OnceCell::new(),
                        cache: OnceCell::new(),
                    })
                    .collect::<Vec<Record>>();

                if !found.is_empty() {
                    return found;
                }
            }
        }

        vec![]
    }

    fn fetch_definition(&self, entry_offset: usize) -> Option<Vec<u8>> {
        let offset = self.record_offset(entry_offset)?;
        let buf = &self.mmap[self.records_start + offset.buf_offset..];
        let (_, decompressed) =
            record_block_parser(offset.record_size, offset.decomp_size)(buf).unwrap();
        let result: IResult<&[u8], &[u8]> =
            take_till(|x| x == 0)(&decompressed[offset.block_offset..]);
        let (_, raw) = result.unwrap();
        Some(raw.to_vec())
    }
}
