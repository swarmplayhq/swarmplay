//! Read-only PlayStation PFS images (`.ffpfs`, `.ffpfsc`), as some PS5 releases ship: a PFS file
//! system holding one exFAT image of the game (`uroot/<TITLE>.exfat`), often compressed in PFSC
//! blocks. Only unencrypted, unsigned images are read. The exFAT image is exposed as an
//! `exfat::Image` that unpacks the blocks it is asked for, so `exfat.rs` reads it as it reads a
//! file. Every offset, count and length is checked against the image, so a damaged or hostile
//! image fails with an error instead of reading outside it.
//!
//! Layout (little-endian): a header in block 0 (magic 0x01332A0B at 8, mode at 0x1C, block size at
//! 0x20, inode count at 0x30, inode blocks at 0x40, superroot inode at 0x48); 0xA8-byte inodes
//! from block 1 (mode, flags at 4, sizes at 8 and 0x10, block count at 0x60, first block at 0x64);
//! directory entries (inode, type, name length, entry size, name) in their folder's blocks.

use tr::tr;
use anyhow::{bail, ensure, Context, Result};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::{fs::File, os::unix::fs::FileExt};

const MAGIC: u64 = 0x0133_2A0B;
const INODE: u64 = 0xA8;
/// Header mode bits: signed (inodes carry signatures), 64-bit inodes, encrypted.
const SIGNED: u16 = 0x1;
const WIDE: u16 = 0x2;
const ENCRYPTED: u16 = 0x4;
/// Inode flag: the file's data is a PFSC container.
const COMPRESSED: u32 = 0x1;
const DIR_FILE: i32 = 2;
const DIR_DIR: i32 = 3;
const DIR_DOT: i32 = 4;
const DIR_DOTDOT: i32 = 5;
const MAX_INODES: u64 = 1_000_000;
const MAX_DIR_BLOCKS: u64 = 64;
/// PFSC block offsets read into memory: 64 MiB, 512 GiB of game in 64 KiB blocks.
const MAX_TABLE: u64 = 64 * 1024 * 1024;
const CACHE: usize = 8;

/// Whether a file starts with a PFS header.
pub fn is_image(file: &File) -> bool {
    let mut b = [0u8; 16];
    file.read_exact_at(&mut b, 0).is_ok() && u64::from_le_bytes(b[8..16].try_into().unwrap()) == MAGIC
}

fn u16_at(b: &[u8], o: usize) -> u16 { u16::from_le_bytes(b[o..o + 2].try_into().unwrap()) }
fn u32_at(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }
fn i32_at(b: &[u8], o: usize) -> i32 { i32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }
fn u64_at(b: &[u8], o: usize) -> u64 { u64::from_le_bytes(b[o..o + 8].try_into().unwrap()) }

struct Inode { directory: bool, regular: bool, flags: u32, size: u64, size2: u64, start: u64, extent: u64 }

struct Pfs { file: File, len: u64, block: u64, inodes: u64 }

impl Pfs {
    fn open(file: File) -> Result<Self> {
        let mut h = [0u8; 0x50];
        file.read_exact_at(&mut h, 0).context("Read PFS header")?;
        ensure!(u64_at(&h, 8) == MAGIC, tr!("Not a PFS image"));
        let mode = u16_at(&h, 0x1C);
        ensure!(mode & ENCRYPTED == 0, tr!("This PFS image is encrypted; only decrypted images can be installed"));
        ensure!(mode & (SIGNED | WIDE) == 0, tr!("Unsupported PFS image (signed or 64-bit inodes)"));
        let block = u32_at(&h, 0x20) as u64;
        ensure!(block.is_power_of_two() && (0x1000..=0x10_0000).contains(&block), "Unsupported PFS block size");
        let (inodes, inode_blocks) = (u64_at(&h, 0x30), u64_at(&h, 0x40));
        ensure!((1..=MAX_INODES).contains(&inodes) && inode_blocks * (block / INODE) >= inodes, "Invalid PFS inode table");
        let len = file.metadata()?.len();
        ensure!((1 + inode_blocks) * block <= len, tr!("The PFS image is shorter than its header says; wait until the download is complete"));
        ensure!(u64_at(&h, 0x48) == 0, "Unsupported PFS image (superroot isn't the first inode)");
        Ok(Self { file, len, block, inodes })
    }

    fn inode(&self, i: u64) -> Result<Inode> {
        ensure!(i < self.inodes, "PFS inode {i} is outside the inode table");
        let per = self.block / INODE;
        let mut b = [0u8; INODE as usize];
        self.file.read_exact_at(&mut b, (1 + i / per) * self.block + (i % per) * INODE).context("Read PFS inode")?;
        let mode = u16_at(&b, 0) as u32 & 0xF000;
        let blocks = u32_at(&b, 0x60) as u64;
        let first = i32_at(&b, 0x64);
        // Files and folders take consecutive blocks from the first; no indirect blocks.
        let contiguous = (1..12).map(|k| i32_at(&b, 0x64 + 4 * k)).all(|d| d == -1 || d == 0)
            && (0..5).all(|k| i32_at(&b, 0x94 + 4 * k) == 0);
        ensure!(contiguous || blocks <= 1, tr!("Unsupported PFS image (fragmented file)"));
        let extent = blocks * self.block;
        let start = if blocks == 0 { 0 } else {
            ensure!(first > 0, "Invalid PFS block number");
            first as u64 * self.block
        };
        ensure!(start.checked_add(extent).is_some_and(|end| end <= self.len), "PFS file runs past the end of the image");
        Ok(Inode { directory: mode == 0x4000, regular: mode == 0x8000, flags: u32_at(&b, 4), size: u64_at(&b, 8), size2: u64_at(&b, 0x10), start, extent })
    }

    /// A folder's entries other than `.` and `..`: (inode, type, name).
    fn entries(&self, i: u64) -> Result<Vec<(u64, i32, String)>> {
        let dir = self.inode(i)?;
        ensure!(dir.directory, "PFS inode {i} isn't a folder");
        ensure!(dir.extent <= MAX_DIR_BLOCKS * self.block, "PFS folder is too large");
        let mut data = vec![0u8; dir.extent as usize];
        self.file.read_exact_at(&mut data, dir.start).context("Read PFS folder")?;
        let mut out = Vec::new();
        for chunk in data.chunks(self.block as usize) {
            let mut o = 0usize;
            while o + 16 <= chunk.len() {
                let (ino, kind, name_len, size) = (i32_at(chunk, o), i32_at(chunk, o + 4), i32_at(chunk, o + 8), i32_at(chunk, o + 12));
                if size == 0 { break; }
                ensure!(ino >= 0 && name_len > 0 && name_len <= 255 && size as i64 >= 16 + name_len as i64 && o + size as usize <= chunk.len(),
                    "Invalid PFS folder entry");
                if kind != DIR_DOT && kind != DIR_DOTDOT {
                    let name = std::str::from_utf8(&chunk[o + 16..o + 16 + name_len as usize]).context("PFS file name isn't UTF-8")?;
                    ensure!(!name.contains('/') && name != "." && name != "..", "Unsafe PFS file name");
                    out.push((ino as u64, kind, name.to_string()));
                }
                o += size as usize;
            }
        }
        Ok(out)
    }
}

/// The game's exFAT image inside a PFS image, ready for `exfat::Volume::open_image`.
pub fn exfat_inside(file: File) -> Result<Box<dyn crate::exfat::Image>> {
    let pfs = Pfs::open(file)?;
    let uroot = pfs.entries(0)?.into_iter().find(|(_, kind, name)| *kind == DIR_DIR && name == "uroot")
        .map(|(ino, _, _)| ino).context("PFS image has no uroot folder")?;
    let entries = pfs.entries(uroot)?;
    let image = match entries.as_slice() {
        [(ino, DIR_FILE, name)] if name.to_ascii_lowercase().ends_with(".exfat") => *ino,
        _ => bail!(tr!("Unsupported PFS image: only one holding a single exFAT game image can be installed")),
    };
    let node = pfs.inode(image)?;
    ensure!(node.regular, "The PFS image's exFAT image isn't a file");
    if node.flags & COMPRESSED == 0 {
        ensure!(node.size <= node.extent, "PFS file is larger than its blocks");
        return Ok(Box::new(Window { file: pfs.file, start: node.start, len: node.size }));
    }
    Ok(Box::new(Pfsc::open(pfs.file, node.start, node.extent, [node.size, node.size2])?))
}

/// A stored (uncompressed) file inside the image.
#[derive(Debug)]
struct Window { file: File, start: u64, len: u64 }

impl crate::exfat::Image for Window {
    fn read_image_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.file.read_exact_at(buf, self.start + offset)
    }
    fn len(&self) -> std::io::Result<u64> { Ok(self.len) }
}

/// A PFSC container: a header (magic "PFSC", block size at 0x0C, offset table at 0x18, data size
/// at 0x28), then for each block where its compressed bytes start (one more for the end). A block
/// as long as the block size is stored as it is; an empty one is zeros; any other is zlib.
#[derive(Debug)]
struct Pfsc { file: File, start: u64, block: u64, offsets: Vec<u64>, len: u64, cache: Mutex<Vec<(u64, Arc<Vec<u8>>)>> }

impl Pfsc {
    /// `sizes`: the two the inode records (the container's and the unpacked file's).
    fn open(file: File, start: u64, extent: u64, sizes: [u64; 2]) -> Result<Self> {
        let mut h = [0u8; 0x30];
        ensure!(extent >= 0x30, "PFSC container is too short");
        file.read_exact_at(&mut h, start).context("Read PFSC header")?;
        ensure!(&h[..4] == b"PFSC", "Compressed PFS file isn't a PFSC container");
        let block = u32_at(&h, 0x0C) as u64;
        ensure!(block.is_power_of_two() && (0x1000..=0x10_0000).contains(&block), "Unsupported PFSC block size");
        let (table, len) = (u64_at(&h, 0x18), u64_at(&h, 0x28));
        ensure!(len > 0 && sizes.contains(&len), "PFSC size differs from the PFS file's");
        let count = len.div_ceil(block);
        let bytes = (count + 1).checked_mul(8).filter(|b| *b <= MAX_TABLE).context("PFSC offset table is too large")?;
        ensure!(table >= 0x30 && table.checked_add(bytes).is_some_and(|end| end <= extent), "PFSC offset table is outside the file");
        let mut raw = vec![0u8; bytes as usize];
        file.read_exact_at(&mut raw, start + table).context("Read PFSC offset table")?;
        let offsets: Vec<u64> = raw.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
        ensure!(offsets[0] >= table + bytes && offsets[count as usize] <= extent, "PFSC blocks are outside the file");
        ensure!(offsets.windows(2).all(|w| w[0] <= w[1] && w[1] - w[0] <= block), "Invalid PFSC block offsets");
        Ok(Self { file, start, block, offsets, len, cache: Mutex::new(Vec::new()) })
    }

    /// Block `i`, unpacked: `block` bytes, fewer for the last.
    fn unpack(&self, i: u64) -> std::io::Result<Arc<Vec<u8>>> {
        if let Some((_, b)) = self.cache.lock().unwrap().iter().find(|(k, _)| *k == i) { return Ok(b.clone()); }
        let want = (self.len - i * self.block).min(self.block) as usize;
        let (from, to) = (self.offsets[i as usize], self.offsets[i as usize + 1]);
        let stored = (to - from) as usize;
        let mut packed = vec![0u8; stored];
        self.file.read_exact_at(&mut packed, self.start + from)?;
        let mut out = Vec::with_capacity(self.block as usize);
        if stored == 0 {
            out.resize(want, 0);
        } else if stored as u64 == self.block {
            out = packed;
        } else {
            flate2::read::ZlibDecoder::new(&packed[..]).take(self.block + 1).read_to_end(&mut out)?;
        }
        if out.len() < want || out.len() as u64 > self.block {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("PFSC block {i} unpacks to {} bytes, not {want}", out.len())));
        }
        out.truncate(want);
        let out = Arc::new(out);
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHE { cache.remove(0); }
        cache.push((i, out.clone()));
        Ok(out)
    }
}

impl crate::exfat::Image for Pfsc {
    fn read_image_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len) {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let mut done = 0usize;
        while done < buf.len() {
            let at = offset + done as u64;
            let block = self.unpack(at / self.block)?;
            let inside = (at % self.block) as usize;
            let take = (block.len() - inside).min(buf.len() - done);
            buf[done..done + take].copy_from_slice(&block[inside..inside + take]);
            done += take;
        }
        Ok(())
    }
    fn len(&self) -> std::io::Result<u64> { Ok(self.len) }
}

/// Test images: a PFS image holding `uroot/<name>`, compressed in PFSC blocks or stored.
#[cfg(test)]
pub mod build {
    use std::io::Write;

    pub const BLOCK: usize = 0x10000;

    fn inode(out: &mut [u8], mode: u16, flags: u32, size: u64, size2: u64, blocks: u32, first: i32) {
        out[0..2].copy_from_slice(&mode.to_le_bytes());
        out[2..4].copy_from_slice(&1u16.to_le_bytes());
        out[4..8].copy_from_slice(&flags.to_le_bytes());
        out[8..16].copy_from_slice(&size.to_le_bytes());
        out[0x10..0x18].copy_from_slice(&size2.to_le_bytes());
        out[0x60..0x64].copy_from_slice(&blocks.to_le_bytes());
        out[0x64..0x68].copy_from_slice(&first.to_le_bytes());
        for k in 1..12 { out[0x64 + 4 * k..0x68 + 4 * k].copy_from_slice(&(-1i32).to_le_bytes()); }
    }

    fn entry(out: &mut Vec<u8>, ino: i32, kind: i32, name: &str) {
        let size = (16 + name.len() + 1).div_ceil(8) * 8;
        let at = out.len();
        out.resize(at + size, 0);
        for (k, v) in [ino, kind, name.len() as i32, size as i32].into_iter().enumerate() {
            out[at + 4 * k..at + 4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        out[at + 16..at + 16 + name.len()].copy_from_slice(name.as_bytes());
    }

    /// A PFSC container of `data` in `block`-byte blocks; incompressible blocks are stored.
    pub fn pfsc(data: &[u8], block: usize) -> Vec<u8> {
        let count = data.len().div_ceil(block);
        let table = 0x400usize;
        let start = table + (count + 1) * 8;
        let mut packed = Vec::new();
        let mut offsets = vec![start as u64];
        for chunk in data.chunks(block) {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(chunk).unwrap();
            let z = z.finish().unwrap();
            if z.len() >= block { let mut raw = chunk.to_vec(); raw.resize(block, 0); packed.extend(raw); } else { packed.extend(z); }
            offsets.push((start + packed.len()) as u64);
        }
        let mut out = vec![0u8; start];
        out[..4].copy_from_slice(b"PFSC");
        out[8..12].copy_from_slice(&6u32.to_le_bytes());
        out[0x0C..0x10].copy_from_slice(&(block as u32).to_le_bytes());
        out[0x10..0x18].copy_from_slice(&(block as u64).to_le_bytes());
        out[0x18..0x20].copy_from_slice(&(table as u64).to_le_bytes());
        out[0x20..0x28].copy_from_slice(&(start as u64).to_le_bytes());
        out[0x28..0x30].copy_from_slice(&(data.len() as u64).to_le_bytes());
        for (k, o) in offsets.iter().enumerate() { out[table + 8 * k..table + 8 * k + 8].copy_from_slice(&o.to_le_bytes()); }
        out.extend(packed);
        out
    }

    /// The image: header, inodes (superroot, flat_path_table, uroot, the file), their blocks.
    pub fn image(name: &str, data: &[u8], compressed: bool) -> Vec<u8> {
        let stored = if compressed { pfsc(data, BLOCK) } else { data.to_vec() };
        let file_blocks = stored.len().div_ceil(BLOCK).max(1);
        let mut img = vec![0u8; BLOCK * 6];
        img[0..8].copy_from_slice(&2u64.to_le_bytes());
        img[8..16].copy_from_slice(&super::MAGIC.to_le_bytes());
        img[0x1A] = 1;
        img[0x1C..0x1E].copy_from_slice(&8u16.to_le_bytes());
        img[0x20..0x24].copy_from_slice(&(BLOCK as u32).to_le_bytes());
        img[0x30..0x38].copy_from_slice(&4u64.to_le_bytes());
        img[0x38..0x40].copy_from_slice(&((5 + file_blocks) as u64).to_le_bytes());
        img[0x40..0x48].copy_from_slice(&1u64.to_le_bytes());
        let inodes = BLOCK;
        let ino = |k: usize| inodes + k * super::INODE as usize..inodes + (k + 1) * super::INODE as usize;
        inode(&mut img[ino(0)], 0x416D, 0x20010, BLOCK as u64, BLOCK as u64, 1, 2);
        inode(&mut img[ino(1)], 0x816D, 0x20010, 8, 8, 1, 3);
        inode(&mut img[ino(2)], 0x416D, 0x10, BLOCK as u64, BLOCK as u64, 1, 4);
        let (size, size2) = if compressed { (stored.len() as u64, data.len() as u64) } else { (data.len() as u64, data.len() as u64) };
        inode(&mut img[ino(3)], 0x816D, if compressed { 0x11 } else { 0x10 }, size, size2, file_blocks as u32, 5);
        let mut root = Vec::new();
        entry(&mut root, 1, 2, "flat_path_table");
        entry(&mut root, 2, 3, "uroot");
        img[2 * BLOCK..2 * BLOCK + root.len()].copy_from_slice(&root);
        let mut uroot = Vec::new();
        entry(&mut uroot, 2, 4, ".");
        entry(&mut uroot, 2, 5, "..");
        entry(&mut uroot, 3, 2, name);
        img[4 * BLOCK..4 * BLOCK + uroot.len()].copy_from_slice(&uroot);
        img.truncate(5 * BLOCK);
        img.extend(&stored);
        img.resize((5 + file_blocks) * BLOCK, 0);
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(bytes: &[u8]) -> (tempfile::NamedTempFile, File) {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, bytes).unwrap();
        let file = File::open(f.path()).unwrap();
        (f, file)
    }

    /// Bytes that compress (repeats) and a block that doesn't, ending mid-block.
    fn payload() -> Vec<u8> {
        let mut data: Vec<u8> = (0..build::BLOCK * 2).map(|i| (i / 300) as u8).collect();
        let mut x = 0x2545F491u32;
        data.extend((0..build::BLOCK).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }));
        data.extend(vec![7u8; 1234]);
        data
    }

    fn read_all(image: &dyn crate::exfat::Image) -> Vec<u8> {
        let mut out = vec![0u8; image.len().unwrap() as usize];
        // Odd-sized reads cross block boundaries.
        for (k, chunk) in out.chunks_mut(40_000).enumerate() {
            image.read_image_at(chunk, k as u64 * 40_000).unwrap();
        }
        out
    }

    #[test]
    fn reads_the_image_inside_compressed_or_stored() {
        let data = payload();
        for compressed in [true, false] {
            let (_f, file) = temp(&build::image("PPSA12345.exfat", &data, compressed));
            assert!(is_image(&file));
            let image = exfat_inside(file).unwrap();
            assert_eq!(read_all(image.as_ref()), data, "compressed: {compressed}");
            let mut past = [0u8; 2];
            assert!(image.read_image_at(&mut past, data.len() as u64 - 1).is_err());
        }
    }

    #[test]
    fn refuses_encrypted_damaged_or_unexpected_images() {
        let data = payload();
        let good = build::image("PPSA12345.exfat", &data, true);
        let open = |bytes: &[u8]| { let (_f, file) = temp(bytes); exfat_inside(file).map(|_| ()) };
        assert!(open(&good).is_ok());
        let mut encrypted = good.clone();
        encrypted[0x1C] |= ENCRYPTED as u8;
        assert!(open(&encrypted).unwrap_err().to_string().contains("encrypted"));
        // Truncated: the file's blocks run past the end.
        assert!(open(&good[..good.len() - build::BLOCK]).is_err());
        // Not a single .exfat inside.
        assert!(open(&build::image("eboot.bin", &data, true)).is_err());
        // A block offset pointing past the container.
        let mut bad = good.clone();
        let table = 5 * build::BLOCK + 0x400;
        bad[table + 8..table + 16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(open(&bad).is_err());
        // A block whose zlib data is damaged fails when read, not silently.
        let mut corrupt = good.clone();
        let first = u64::from_le_bytes(good[table..table + 8].try_into().unwrap()) as usize;
        corrupt[5 * build::BLOCK + first + 4] ^= 0xFF;
        let (_f, file) = temp(&corrupt);
        let image = exfat_inside(file).unwrap();
        let mut buf = vec![0u8; 16];
        assert!(image.read_image_at(&mut buf, 0).is_err());
    }

    /// A real release: `AZRAK_PFS_SAMPLE=/path/to/game.ffpfsc cargo test --release -- --ignored real_pfs_image`.
    /// Lists the game and unpacks every file in it.
    #[test]
    #[ignore]
    fn real_pfs_image() {
        let path = std::env::var("AZRAK_PFS_SAMPLE").expect("AZRAK_PFS_SAMPLE");
        let volume = crate::exfat::Volume::open_image(exfat_inside(File::open(path).unwrap()).unwrap()).unwrap();
        let nodes = volume.list(&|| Ok(())).unwrap();
        let param = nodes.iter().find(|n| n.path.ends_with("sce_sys/param.json")).expect("sce_sys/param.json");
        let mut json = Vec::new();
        volume.copy(param, &mut |c| { json.extend_from_slice(c); Ok(()) }).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
        println!("titleId {} · {} files", json["titleId"], nodes.iter().filter(|n| !n.directory).count());
        assert!(nodes.iter().any(|n| n.path.ends_with("eboot.bin") && n.size > 0));
        let (mut total, mut read) = (0u64, 0u64);
        for node in nodes.iter().filter(|n| !n.directory) {
            total += node.size;
            volume.copy(node, &mut |c| { read += c.len() as u64; Ok(()) }).unwrap();
        }
        assert_eq!(read, total);
        println!("unpacked {total} bytes");
    }
}
